//! The daemon core: the host-side authority over sessions and live runs.
//!
//! [`Daemon`] owns the [`RecordStore`] and a registry of in-flight executions
//! keyed by execution id. Entries are [`ExecutionHandle`]s, so the same
//! machinery drives executions the daemon started itself (local PTY runs) and
//! ones owned by a shim over IPC (see [`crate::ipc`]). The gRPC layer
//! ([`crate::service`]) is a thin adapter over this.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cterm_core::{
    Command, Error, ExecutionEvent, ExecutionId, ExecutionRecord, Host, RecordStore, Result,
    Session, SessionId,
};
use cterm_proto::v1;
use cterm_shim::{start_recorded, RunSpec};
use tokio::sync::broadcast;

use crate::handle::ExecutionHandle;

/// Registry of live executions and their control handles.
pub(crate) type Registry = Arc<Mutex<HashMap<ExecutionId, Arc<dyn ExecutionHandle>>>>;

/// The host-side daemon.
pub struct Daemon<S: RecordStore> {
    pub(crate) store: Arc<S>,
    pub(crate) registry: Registry,
}

impl<S: RecordStore> Clone for Daemon<S> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            registry: self.registry.clone(),
        }
    }
}

/// Outcome of starting a run: the new execution id and a receiver that captured
/// the live event stream from the very first event.
pub struct Started {
    /// The started execution's id.
    pub execution: ExecutionId,
    /// Live events, starting at `Started`.
    pub events: broadcast::Receiver<ExecutionEvent>,
}

impl<S: RecordStore + Send + Sync + 'static> Daemon<S> {
    /// Builds a daemon over the given store.
    pub fn new(store: Arc<S>) -> Self {
        Self {
            store,
            registry: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// The backing store (used by the gRPC layer for list/get).
    #[must_use]
    pub fn store(&self) -> &Arc<S> {
        &self.store
    }

    /// Registers a live execution handle.
    pub(crate) fn register(&self, execution: ExecutionId, handle: Arc<dyn ExecutionHandle>) {
        self.registry
            .lock()
            .expect("registry poisoned")
            .insert(execution, handle);
    }

    /// Removes a live execution handle.
    pub(crate) fn deregister(&self, execution: ExecutionId) {
        self.registry
            .lock()
            .expect("registry poisoned")
            .remove(&execution);
    }

    /// Starts a command under a PTY on this host and begins recording it.
    ///
    /// If `req.session` is empty a new local session is opened. Defaults:
    /// program `/bin/bash`, args `["-lc", <command>]`, 24x80.
    pub async fn run(&self, req: v1::RunRequest) -> Result<Started> {
        let session = self.resolve_session(&req).await?;

        let mut command = Command::new(req.command.clone(), "bash");
        if !req.cwd.is_empty() {
            command = command.with_cwd(req.cwd.clone());
        }

        let program = if req.program.is_empty() {
            "/bin/bash".to_owned()
        } else {
            req.program.clone()
        };
        let args: Vec<String> = if req.args.is_empty() {
            vec!["-lc".to_owned(), req.command.clone()]
        } else {
            req.args.clone()
        };
        let mut spec = RunSpec::new(program, args);
        spec.cwd = (!req.cwd.is_empty()).then(|| req.cwd.clone());
        spec.rows = if req.rows == 0 { 24 } else { req.rows as u16 };
        spec.cols = if req.cols == 0 { 80 } else { req.cols as u16 };

        let mut run = start_recorded(self.store.clone(), session, command, spec).await?;
        let execution = run.execution();
        let events = run
            .take_primary()
            .expect("primary available on a fresh run");

        self.register(execution, Arc::new(run.control()));

        // Deregister when the run reaches a terminal status, even if the client
        // that started it has gone away.
        let daemon = self.clone();
        let done = run.done;
        tokio::spawn(async move {
            let _ = done.await;
            daemon.deregister(execution);
        });

        Ok(Started { execution, events })
    }

    /// Writes input to a live execution.
    pub async fn send_input(&self, execution: ExecutionId, data: Vec<u8>) -> Result<()> {
        self.handle(execution)?.send_input(data).await
    }

    /// Interrupts a live execution.
    pub async fn interrupt(&self, execution: ExecutionId) -> Result<()> {
        self.handle(execution)?.interrupt().await
    }

    /// Subscribes to the live events of a currently-running execution, if any.
    #[must_use]
    pub fn subscribe(&self, execution: ExecutionId) -> Option<broadcast::Receiver<ExecutionEvent>> {
        self.registry
            .lock()
            .expect("registry poisoned")
            .get(&execution)
            .map(|h| h.subscribe())
    }

    /// All known sessions, most recent first.
    pub async fn list_sessions(&self) -> Result<Vec<Session>> {
        self.store.list_sessions().await
    }

    /// Executions in a session, oldest first.
    pub async fn list_executions(&self, session: SessionId) -> Result<Vec<ExecutionRecord>> {
        self.store.list_executions(session).await
    }

    /// A single execution record.
    pub async fn get_execution(&self, execution: ExecutionId) -> Result<Option<ExecutionRecord>> {
        self.store.get_execution(execution).await
    }

    /// Loads stored history for replay.
    pub async fn history(
        &self,
        execution: ExecutionId,
    ) -> Result<Option<(ExecutionRecord, Vec<cterm_core::StreamChunk>)>> {
        let Some(record) = self.store.get_execution(execution).await? else {
            return Ok(None);
        };
        let chunks = self.store.load_chunks(execution).await?;
        Ok(Some((record, chunks)))
    }

    fn handle(&self, execution: ExecutionId) -> Result<Arc<dyn ExecutionHandle>> {
        self.registry
            .lock()
            .expect("registry poisoned")
            .get(&execution)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("no live execution {execution}")))
    }

    async fn resolve_session(&self, req: &v1::RunRequest) -> Result<SessionId> {
        if req.session.is_empty() {
            let shell = if req.program.is_empty() {
                "bash"
            } else {
                &req.program
            };
            let session = Session::open(Host::local(), shell);
            self.store.put_session(&session).await?;
            Ok(session.id)
        } else {
            req.session
                .parse::<SessionId>()
                .map_err(|e| Error::Other(format!("invalid session id {:?}: {e}", req.session)))
        }
    }
}
