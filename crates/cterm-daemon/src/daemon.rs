//! The daemon core: the host-side authority over sessions and live runs.
//!
//! [`Daemon`] owns the [`RecordStore`] and a registry of in-flight runs keyed by
//! execution id. Starting a run records it (via [`cterm_shim::start_recorded`]),
//! registers a [`RunControl`] so later unary RPCs can reach it, and arranges for
//! the entry to be removed when the run ends. The gRPC layer ([`crate::service`])
//! is a thin adapter over this.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cterm_core::{
    Command, Error, ExecutionEvent, ExecutionId, ExecutionRecord, Host, RecordStore, Result,
    Session, SessionId,
};
use cterm_proto::v1;
use cterm_shim::{start_recorded, RunControl, RunSpec};
use tokio::sync::broadcast;

/// Outcome of starting a run: the new execution id and a receiver that captured
/// the live event stream from the very first event.
pub struct Started {
    /// The started execution's id.
    pub execution: ExecutionId,
    /// Live events, starting at `Started`.
    pub events: broadcast::Receiver<ExecutionEvent>,
}

/// The host-side daemon.
pub struct Daemon<S: RecordStore> {
    store: Arc<S>,
    registry: Arc<Mutex<HashMap<ExecutionId, RunControl<S>>>>,
}

impl<S: RecordStore> Clone for Daemon<S> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            registry: self.registry.clone(),
        }
    }
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

    /// Starts a command under a PTY and begins recording it.
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
        let events = run.take_primary().expect("primary available on a fresh run");

        self.registry
            .lock()
            .expect("registry poisoned")
            .insert(execution, run.control());

        // Deregister when the run reaches a terminal status, even if the client
        // that started it has gone away.
        let registry = self.registry.clone();
        let done = run.done;
        tokio::spawn(async move {
            let _ = done.await;
            registry.lock().expect("registry poisoned").remove(&execution);
        });

        Ok(Started { execution, events })
    }

    /// Writes input to a running execution.
    pub async fn send_input(&self, execution: ExecutionId, data: Vec<u8>) -> Result<()> {
        let control = self.control(execution)?;
        control.send_input(data).await
    }

    /// Interrupts a running execution.
    pub async fn interrupt(&self, execution: ExecutionId) -> Result<()> {
        let control = self.control(execution)?;
        control.interrupt().await
    }

    /// Subscribes to the live events of a currently-running execution, if any.
    #[must_use]
    pub fn subscribe(&self, execution: ExecutionId) -> Option<broadcast::Receiver<ExecutionEvent>> {
        self.registry
            .lock()
            .expect("registry poisoned")
            .get(&execution)
            .map(RunControl::subscribe)
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

    fn control(&self, execution: ExecutionId) -> Result<RunControl<S>> {
        self.registry
            .lock()
            .expect("registry poisoned")
            .get(&execution)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("no running execution {execution}")))
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
