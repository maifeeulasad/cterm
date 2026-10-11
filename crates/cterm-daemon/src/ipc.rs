//! The IPC server: accepts shim connections over a Unix socket, integrates each
//! shim-owned execution into the daemon's registry, persists its events, relays
//! them to gRPC subscribers, and forwards control (input/interrupt) back.
//!
//! This is what makes the hybrid flow live: a human opens an interactive shell
//! through the wrapper on the host; the wrapper connects here; an agent then
//! observes and drives that shell over gRPC, never opening its own connection.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use cterm_core::{
    Error, ExecutionEvent, ExecutionId, ExecutionRecord, ExecutionStatus, RecordStore, Result,
    SessionId, SessionStatus,
};
use cterm_ipc::{read_msg, write_msg, DaemonToShim, ShimToDaemon};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc};

use crate::daemon::Daemon;
use crate::handle::ExecutionHandle;

const EVENT_BUFFER: usize = 1024;

/// Control handle for a remote (shim-owned) execution. Subscribers read a local
/// broadcast of events relayed from the shim; input/interrupt are sent back to
/// the shim over its connection.
pub(crate) struct RemoteControl {
    execution: ExecutionId,
    events: broadcast::Sender<ExecutionEvent>,
    to_shim: mpsc::UnboundedSender<DaemonToShim>,
}

#[async_trait]
impl ExecutionHandle for RemoteControl {
    fn subscribe(&self) -> broadcast::Receiver<ExecutionEvent> {
        self.events.subscribe()
    }

    async fn send_input(&self, data: Vec<u8>) -> Result<()> {
        self.to_shim
            .send(DaemonToShim::Input {
                execution: self.execution,
                data,
            })
            .map_err(|_| Error::Transport("shim disconnected".to_owned()))
    }

    async fn interrupt(&self) -> Result<()> {
        self.to_shim
            .send(DaemonToShim::Interrupt {
                execution: self.execution,
            })
            .map_err(|_| Error::Transport("shim disconnected".to_owned()))
    }
}

impl<S: RecordStore + Send + Sync + 'static> Daemon<S> {
    /// Serves the IPC socket at `path`, accepting shim connections forever.
    /// Replaces any stale socket file at that path.
    pub async fn serve_ipc(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path)
            .map_err(|e| Error::Transport(format!("bind {}: {e}", path.display())))?;
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let daemon = self.clone();
                    tokio::spawn(async move {
                        let _ = daemon.handle_connection(stream).await;
                    });
                }
                Err(e) => return Err(Error::Transport(format!("accept: {e}"))),
            }
        }
    }

    /// Handles one shim connection to completion.
    async fn handle_connection(&self, stream: UnixStream) -> Result<()> {
        let (mut reader, write_half) = stream.into_split();

        // Writer task: drains control messages to the shim.
        let (to_shim_tx, mut to_shim_rx) = mpsc::unbounded_channel::<DaemonToShim>();
        let writer = tokio::spawn(async move {
            let mut w = write_half;
            while let Some(msg) = to_shim_rx.recv().await {
                if write_msg(&mut w, &msg).await.is_err() {
                    break;
                }
            }
        });

        let mut session_id: Option<SessionId> = None;
        let mut events_by_exec: HashMap<ExecutionId, broadcast::Sender<ExecutionEvent>> =
            HashMap::new();
        let mut records: HashMap<ExecutionId, ExecutionRecord> = HashMap::new();

        loop {
            let msg: Option<ShimToDaemon> = read_msg(&mut reader)
                .await
                .map_err(|e| Error::Transport(e.to_string()))?;
            let Some(msg) = msg else {
                break; // clean EOF
            };
            match msg {
                ShimToDaemon::Hello { version: _, session } => {
                    eprintln!(
                        "cterm-daemon: shim connected (session {}, {})",
                        session.id, session.shell
                    );
                    session_id = Some(session.id);
                    let _ = self.store.put_session(&session).await;
                }
                ShimToDaemon::Started { record } => {
                    let _ = self.store.put_execution(&record).await;
                    let (tx, _rx) = broadcast::channel(EVENT_BUFFER);
                    let control = RemoteControl {
                        execution: record.id,
                        events: tx.clone(),
                        to_shim: to_shim_tx.clone(),
                    };
                    self.register(record.id, Arc::new(control));
                    let _ = tx.send(ExecutionEvent::Started(record.clone()));
                    events_by_exec.insert(record.id, tx);
                    records.insert(record.id, record);
                }
                ShimToDaemon::Chunk { chunk } => {
                    let _ = self.store.append_chunk(&chunk).await;
                    if let Some(tx) = events_by_exec.get(&chunk.execution) {
                        let _ = tx.send(ExecutionEvent::Chunk(chunk));
                    }
                }
                ShimToDaemon::Status { execution, status } => {
                    if let Some(rec) = records.get_mut(&execution) {
                        rec.transition(status);
                        let _ = self.store.put_execution(rec).await;
                    }
                    if let Some(tx) = events_by_exec.get(&execution) {
                        let _ = tx.send(ExecutionEvent::Status(status));
                    }
                    self.deregister(execution);
                    events_by_exec.remove(&execution);
                    records.remove(&execution);
                }
                ShimToDaemon::SessionClosed { session, status } => {
                    if let Ok(Some(mut s)) = self.store.get_session(session).await {
                        s.close(status);
                        let _ = self.store.put_session(&s).await;
                    }
                }
            }
        }

        // Connection gone: fail any still-live executions, mark session dropped.
        for (exec, tx) in events_by_exec.drain() {
            self.deregister(exec);
            if let Some(mut rec) = records.remove(&exec) {
                rec.transition(ExecutionStatus::Failed);
                let _ = self.store.put_execution(&rec).await;
            }
            let _ = tx.send(ExecutionEvent::Status(ExecutionStatus::Failed));
        }
        if let Some(sid) = session_id {
            if let Ok(Some(mut s)) = self.store.get_session(sid).await {
                if s.status.is_open() {
                    s.close(SessionStatus::Dropped);
                    let _ = self.store.put_session(&s).await;
                }
            }
        }
        drop(to_shim_tx);
        let _ = writer.await;
        Ok(())
    }
}
