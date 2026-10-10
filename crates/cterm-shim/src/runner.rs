//! Running a command under a PTY *while recording it* to a [`RecordStore`].
//!
//! This is the bridge between [`crate::pty`] and the domain: it opens an
//! [`ExecutionRecord`], drives it through Running to a terminal status, streams
//! the PTY's output into the store as ordered [`StreamChunk`]s, and records any
//! input sent to the process. The daemon reuses this for its agent-facing
//! `Run` capability; it is also usable standalone for a local (no-IPC) mode.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use cterm_core::{
    Command, ExecutionId, ExecutionRecord, ExecutionStatus, RecordStore, Result, SessionId,
    StreamChunk, StreamKind,
};
use tokio::task::JoinHandle;

use crate::pty::{PtySession, Spawn};

/// How to launch the process, with owned fields so it can cross into a task.
#[derive(Clone, Debug)]
pub struct RunSpec {
    /// Program to execute, e.g. `/bin/bash`.
    pub program: String,
    /// Arguments to the program.
    pub args: Vec<String>,
    /// Working directory, or `None` to inherit.
    pub cwd: Option<String>,
    /// Terminal rows.
    pub rows: u16,
    /// Terminal cols.
    pub cols: u16,
}

impl RunSpec {
    /// A spec with a default 24x80 terminal and inherited cwd.
    #[must_use]
    pub fn new(program: impl Into<String>, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            cwd: None,
            rows: 24,
            cols: 80,
        }
    }
}

/// A live, recording execution. Await [`RecordedRun::done`] for the final
/// status; call [`RecordedRun::send_input`] / [`RecordedRun::interrupt`] while
/// it runs.
pub struct RecordedRun<S: RecordStore> {
    execution: ExecutionId,
    pty: Arc<PtySession>,
    store: Arc<S>,
    seq: Arc<AtomicU64>,
    /// Completes when the execution reaches a terminal status.
    pub done: JoinHandle<Result<ExecutionStatus>>,
}

impl<S: RecordStore + 'static> RecordedRun<S> {
    /// This run's execution id.
    #[must_use]
    pub fn execution(&self) -> ExecutionId {
        self.execution
    }

    /// Writes input to the process and records it as a `Stdin` chunk, using the
    /// same sequence space as output so audit ordering reflects real interleaving.
    pub async fn send_input(&self, data: Vec<u8>) -> Result<()> {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        self.store
            .append_chunk(&StreamChunk::now(
                self.execution,
                StreamKind::Stdin,
                seq,
                data.clone(),
            ))
            .await?;
        self.pty.write_input(data).await
    }

    /// Requests interruption (Ctrl-C) of the running process.
    pub async fn interrupt(&self) -> Result<()> {
        self.pty.interrupt().await
    }
}

/// Starts `spec` under a PTY in `session`, recording everything to `store`.
///
/// Returns immediately with a [`RecordedRun`]; the recording loop runs in a
/// background task. The execution is persisted as `Pending`, then `Running`,
/// then its terminal status.
pub async fn start_recorded<S>(
    store: Arc<S>,
    session: SessionId,
    command: Command,
    spec: RunSpec,
) -> Result<RecordedRun<S>>
where
    S: RecordStore + Send + Sync + 'static,
{
    let mut record = ExecutionRecord::open(session, command);
    let execution = record.id;
    store.put_execution(&record).await?;

    // Build a borrowing Spawn just for the (synchronous) spawn call.
    let arg_refs: Vec<&str> = spec.args.iter().map(String::as_str).collect();
    let (pty, mut rx) = PtySession::spawn(Spawn {
        program: &spec.program,
        args: &arg_refs,
        cwd: spec.cwd.as_deref(),
        env: &[],
        rows: spec.rows,
        cols: spec.cols,
    })?;
    let pty = Arc::new(pty);

    record.transition(ExecutionStatus::Running);
    store.put_execution(&record).await?;

    let seq = Arc::new(AtomicU64::new(0));
    let task_store = store.clone();
    let task_pty = pty.clone();
    let task_seq = seq.clone();
    let done = tokio::spawn(async move {
        // Drain output until EOF (child exited, terminal released).
        while let Some(data) = rx.recv().await {
            let s = task_seq.fetch_add(1, Ordering::SeqCst);
            task_store
                .append_chunk(&StreamChunk::now(execution, StreamKind::Stdout, s, data))
                .await?;
        }
        let status = task_pty.wait().await?;
        record.transition(status);
        task_store.put_execution(&record).await?;
        Ok(status)
    });

    Ok(RecordedRun {
        execution,
        pty,
        store,
        seq,
        done,
    })
}
