//! Running a command under a PTY *while recording it* to a [`RecordStore`] and
//! publishing live [`ExecutionEvent`]s.
//!
//! This is the bridge between [`crate::pty`] and the domain. It opens an
//! [`ExecutionRecord`], drives it through Running to a terminal status, streams
//! the PTY's output into the store as ordered [`StreamChunk`]s, records any
//! input sent to the process, and emits every event on a broadcast channel so
//! the daemon can both persist (reliably, inline) and fan out to live
//! subscribers (agents, webview). Storage is authoritative; the broadcast is
//! best-effort and may lag.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use cterm_core::{
    Command, Error, ExecutionEvent, ExecutionId, ExecutionRecord, ExecutionStatus, RecordStore,
    Result, SessionId, StreamChunk, StreamKind,
};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use crate::pty::{PtySession, Spawn};

/// Capacity of each execution's live-event broadcast buffer. Slow subscribers
/// that fall this far behind receive a `Lagged` notice and resync from storage.
pub const EVENT_BUFFER: usize = 1024;

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
    pub fn new(
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            cwd: None,
            rows: 24,
            cols: 80,
        }
    }
}

/// A cloneable handle to control and observe a running execution: send input,
/// interrupt, and subscribe to live events. The daemon keeps one of these per
/// live execution so unary RPCs (`SendInput`, `Interrupt`) and stream RPCs can
/// reach a run started elsewhere.
pub struct RunControl<S: RecordStore> {
    execution: ExecutionId,
    pty: Arc<PtySession>,
    store: Arc<S>,
    seq: Arc<AtomicU64>,
    events: broadcast::Sender<ExecutionEvent>,
}

// Manual Clone: derive would demand `S: Clone`, but we only hold `Arc<S>`.
impl<S: RecordStore> Clone for RunControl<S> {
    fn clone(&self) -> Self {
        Self {
            execution: self.execution,
            pty: self.pty.clone(),
            store: self.store.clone(),
            seq: self.seq.clone(),
            events: self.events.clone(),
        }
    }
}

impl<S: RecordStore + 'static> RunControl<S> {
    /// This run's execution id.
    #[must_use]
    pub fn execution(&self) -> ExecutionId {
        self.execution
    }

    /// Subscribes to the live event stream. Only events emitted after this call
    /// are delivered; replay earlier history from the store if needed.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<ExecutionEvent> {
        self.events.subscribe()
    }

    /// Writes input to the process, records it as a `Stdin` chunk, and publishes
    /// it. Uses the same sequence space as output so ordering reflects reality.
    pub async fn send_input(&self, data: Vec<u8>) -> Result<()> {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        let chunk = StreamChunk::now(self.execution, StreamKind::Stdin, seq, data.clone());
        self.store.append_chunk(&chunk).await?;
        let _ = self.events.send(ExecutionEvent::Chunk(chunk));
        self.pty.write_input(data).await
    }

    /// Requests interruption (Ctrl-C) of the running process.
    pub async fn interrupt(&self) -> Result<()> {
        self.pty.interrupt().await
    }

    /// Resizes the terminal (e.g. on the controlling window's SIGWINCH).
    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        self.pty.resize(rows, cols)
    }
}

/// A live, recording execution. Await [`RecordedRun::done`] for the final
/// status; drive it through its [`control`](RecordedRun::control) handle.
pub struct RecordedRun<S: RecordStore> {
    control: RunControl<S>,
    /// The channel's original receiver, which captured events from creation
    /// (including `Started`). Handed to the first/primary observer.
    primary: Option<broadcast::Receiver<ExecutionEvent>>,
    /// Completes when the execution reaches a terminal status.
    pub done: JoinHandle<Result<ExecutionStatus>>,
}

impl<S: RecordStore + 'static> RecordedRun<S> {
    /// This run's execution id.
    #[must_use]
    pub fn execution(&self) -> ExecutionId {
        self.control.execution
    }

    /// A cloneable control handle for this run.
    #[must_use]
    pub fn control(&self) -> RunControl<S> {
        self.control.clone()
    }

    /// Takes the primary receiver, which captured every event from the start
    /// (including `Started`). Available once; later observers use
    /// [`subscribe`](RecordedRun::subscribe) and resync history from the store.
    pub fn take_primary(&mut self) -> Option<broadcast::Receiver<ExecutionEvent>> {
        self.primary.take()
    }

    /// Subscribes to the live event stream (see [`RunControl::subscribe`]).
    /// Only events after this call are delivered.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<ExecutionEvent> {
        self.control.subscribe()
    }

    /// Sends input to the process (see [`RunControl::send_input`]).
    pub async fn send_input(&self, data: Vec<u8>) -> Result<()> {
        self.control.send_input(data).await
    }

    /// Interrupts the process (see [`RunControl::interrupt`]).
    pub async fn interrupt(&self) -> Result<()> {
        self.control.interrupt().await
    }

    /// Resizes the terminal (see [`RunControl::resize`]).
    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        self.control.resize(rows, cols)
    }
}

/// Starts `spec` under a PTY in `session`, recording everything to `store` and
/// publishing live events.
///
/// Returns immediately; the recording loop runs in a background task. The
/// execution is persisted as `Pending`, then `Running` (at which point a
/// [`ExecutionEvent::Started`] is published), then its terminal status.
///
/// To observe from the very first event (`Started`), take
/// [`RecordedRun::take_primary`] on the returned handle; additional observers
/// use [`RecordedRun::subscribe`] and resync earlier history from the store.
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

    // The channel's original receiver (`primary`) captures all events from here
    // on, including the `Started` sent just below.
    let (events, primary) = broadcast::channel(EVENT_BUFFER);
    let seq = Arc::new(AtomicU64::new(0));
    let control = RunControl {
        execution,
        pty: pty.clone(),
        store: store.clone(),
        seq: seq.clone(),
        events: events.clone(),
    };

    // Announce the running execution before spawning the drain loop, so a
    // subscriber taken from the returned handle sees Started first.
    let _ = events.send(ExecutionEvent::Started(record.clone()));

    let done = tokio::spawn(async move {
        // Completion is driven by the child exiting, not by output EOF: on
        // Windows/ConPTY the output pipe does not reliably reach EOF when the
        // child dies, so we wait on the child and drain output alongside it.
        let wait_pty = pty.clone();
        let mut waiter = tokio::spawn(async move { wait_pty.wait().await });

        let record_chunk = |data: Vec<u8>| {
            let s = seq.fetch_add(1, Ordering::SeqCst);
            StreamChunk::now(execution, StreamKind::Stdout, s, data)
        };

        let status = loop {
            tokio::select! {
                biased;
                maybe = rx.recv() => match maybe {
                    Some(data) => {
                        let chunk = record_chunk(data);
                        store.append_chunk(&chunk).await?;
                        let _ = events.send(ExecutionEvent::Chunk(chunk));
                    }
                    // Output reached EOF (Unix case): the child is finished.
                    None => break waiter
                        .await
                        .map_err(|e| Error::Other(format!("wait join failed: {e}")))??,
                },
                res = &mut waiter => {
                    let status = res
                        .map_err(|e| Error::Other(format!("wait join failed: {e}")))??;
                    // Flush any output already buffered after the child exited.
                    while let Ok(Some(data)) =
                        tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await
                    {
                        let chunk = record_chunk(data);
                        store.append_chunk(&chunk).await?;
                        let _ = events.send(ExecutionEvent::Chunk(chunk));
                    }
                    break status;
                }
            }
        };

        record.transition(status);
        store.put_execution(&record).await?;
        let _ = events.send(ExecutionEvent::Status(status));
        Ok(status)
    });

    Ok(RecordedRun {
        control,
        primary: Some(primary),
        done,
    })
}
