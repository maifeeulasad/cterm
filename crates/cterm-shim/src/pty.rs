//! A single process running under a pseudo-terminal.
//!
//! [`PtySession`] spawns a program (normally the real shell) attached to a PTY,
//! so the program behaves exactly as if a human were at a terminal: it sees a
//! tty, line editing and job control work, and `isatty()` is true. We stream
//! its output, feed it input, deliver interrupts with real Ctrl-C semantics,
//! and report how it ended as a [`cterm_core::ExecutionStatus`].
//!
//! `portable-pty` is deliberately blocking; we bridge it to async by moving the
//! read loop onto a dedicated OS thread (output arrives on an mpsc channel) and
//! running writes / the final `wait()` on blocking tasks.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use cterm_core::{Error, ExecutionStatus, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use tokio::sync::mpsc;

/// Raw bytes read from the PTY, as produced by the read loop.
pub type OutputReceiver = mpsc::UnboundedReceiver<Vec<u8>>;

/// How a program should be launched under a PTY.
#[derive(Clone, Debug)]
pub struct Spawn<'a> {
    /// Program to execute, e.g. `/bin/bash`.
    pub program: &'a str,
    /// Arguments passed to the program.
    pub args: &'a [&'a str],
    /// Working directory, or `None` to inherit.
    pub cwd: Option<&'a str>,
    /// Extra environment variables layered on top of the inherited environment.
    pub env: &'a [(&'a str, &'a str)],
    /// Initial terminal size.
    pub rows: u16,
    /// Initial terminal size.
    pub cols: u16,
}

impl<'a> Spawn<'a> {
    /// A command with a default 24x80 terminal and inherited cwd/env.
    #[must_use]
    pub fn command(program: &'a str, args: &'a [&'a str]) -> Self {
        Self {
            program,
            args,
            cwd: None,
            env: &[],
            rows: 24,
            cols: 80,
        }
    }
}

/// A live PTY-hosted process.
///
/// Cloneable handle to control operations is intentionally *not* provided; the
/// session owns its child. Share it behind an `Arc` if several tasks need it.
pub struct PtySession {
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    child: Arc<Mutex<Option<Box<dyn Child + Send + Sync>>>>,
    pid: Option<u32>,
    interrupted: Arc<AtomicBool>,
}

impl PtySession {
    /// Spawns `spec` under a fresh PTY, returning the session and a receiver of
    /// its output bytes. The receiver closes (yields `None`) at end-of-output,
    /// which happens once the child has exited and released the terminal.
    pub fn spawn(spec: Spawn<'_>) -> Result<(Self, OutputReceiver)> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: spec.rows,
                cols: spec.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| Error::Other(format!("openpty failed: {e}")))?;

        let mut cmd = CommandBuilder::new(spec.program);
        cmd.args(spec.args);
        if let Some(cwd) = spec.cwd {
            cmd.cwd(cwd);
        }
        for (k, v) in spec.env {
            cmd.env(k, v);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| Error::Other(format!("spawn failed: {e}")))?;
        let pid = child.process_id();

        // Drop our handle to the slave so that, once the child exits and closes
        // its own descriptors, the master read side reaches EOF. Without this
        // the read loop would block forever.
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| Error::Other(format!("clone reader failed: {e}")))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| Error::Other(format!("take writer failed: {e}")))?;

        let (tx, rx) = mpsc::unbounded_channel();
        spawn_read_loop(reader, tx);

        Ok((
            Self {
                master: Arc::new(Mutex::new(pair.master)),
                writer: Arc::new(Mutex::new(writer)),
                child: Arc::new(Mutex::new(Some(child))),
                pid,
                interrupted: Arc::new(AtomicBool::new(false)),
            },
            rx,
        ))
    }

    /// The child's process id, if the platform reports one.
    #[must_use]
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Writes bytes to the terminal's input (as if typed).
    pub async fn write_input(&self, data: Vec<u8>) -> Result<()> {
        let writer = self.writer.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut w = writer
                .lock()
                .map_err(|_| Error::Transport("pty writer poisoned".to_owned()))?;
            w.write_all(&data)
                .and_then(|()| w.flush())
                .map_err(|e| Error::Transport(e.to_string()))
        })
        .await
        .map_err(|e| Error::Transport(format!("write task join failed: {e}")))?
    }

    /// Resizes the terminal (SIGWINCH to the child).
    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        let master = self
            .master
            .lock()
            .map_err(|_| Error::Transport("pty master poisoned".to_owned()))?;
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| Error::Transport(e.to_string()))
    }

    /// Delivers an interrupt (Ctrl-C). On Unix this sends `SIGINT` to the
    /// child's process group, matching what a terminal does for a foreground
    /// job. The session remembers that it initiated the interrupt so [`wait`]
    /// can report [`ExecutionStatus::Interrupted`].
    ///
    /// [`wait`]: PtySession::wait
    pub async fn interrupt(&self) -> Result<()> {
        self.interrupted.store(true, Ordering::SeqCst);
        self.signal_group(Interrupt::Int)
    }

    /// Delivers `SIGTERM` to the child's process group (graceful stop).
    pub async fn terminate(&self) -> Result<()> {
        self.interrupted.store(true, Ordering::SeqCst);
        self.signal_group(Interrupt::Term)
    }

    #[cfg(unix)]
    fn signal_group(&self, which: Interrupt) -> Result<()> {
        use nix::sys::signal::{killpg, Signal};
        use nix::unistd::Pid;
        let Some(pid) = self.pid else {
            return Ok(());
        };
        let sig = match which {
            Interrupt::Int => Signal::SIGINT,
            Interrupt::Term => Signal::SIGTERM,
        };
        // The child is a session/group leader (the PTY made it so), so its pid
        // is also its process-group id.
        killpg(Pid::from_raw(pid as i32), sig).map_err(|e| Error::Transport(e.to_string()))
    }

    #[cfg(not(unix))]
    fn signal_group(&self, _which: Interrupt) -> Result<()> {
        // Windows: delivering Ctrl-C to a specific child is handled separately
        // when the Windows backend lands.
        Err(Error::Transport(
            "interrupt is not yet implemented on this platform".to_owned(),
        ))
    }

    /// Waits for the child to exit and reports how it ended.
    ///
    /// If an interrupt/terminate was requested through this session, the result
    /// is [`ExecutionStatus::Interrupted`]; otherwise it is
    /// [`ExecutionStatus::Completed`] with the exit code. May be called once.
    pub async fn wait(&self) -> Result<ExecutionStatus> {
        let child = self.child.clone();
        let interrupted = self.interrupted.clone();
        let status = tokio::task::spawn_blocking(move || -> Result<u32> {
            let mut guard = child
                .lock()
                .map_err(|_| Error::Other("pty child poisoned".to_owned()))?;
            let mut c = guard
                .take()
                .ok_or_else(|| Error::Other("child already waited".to_owned()))?;
            let status = c.wait().map_err(|e| Error::Other(e.to_string()))?;
            Ok(status.exit_code())
        })
        .await
        .map_err(|e| Error::Other(format!("wait task join failed: {e}")))??;

        if interrupted.load(Ordering::SeqCst) {
            Ok(ExecutionStatus::Interrupted {
                signal: Some(SIGINT_NUMBER),
            })
        } else {
            Ok(ExecutionStatus::Completed {
                code: status as i32,
            })
        }
    }
}

/// POSIX signal number for SIGINT; reported in interrupted status.
const SIGINT_NUMBER: i32 = 2;

#[derive(Clone, Copy)]
enum Interrupt {
    Int,
    Term,
}

/// Moves the blocking reader onto its own thread, forwarding chunks until EOF.
fn spawn_read_loop(mut reader: Box<dyn Read + Send>, tx: mpsc::UnboundedSender<Vec<u8>>) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break, // EOF: child gone, terminal released.
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break; // Receiver dropped; stop reading.
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        // Dropping `tx` here closes the channel, signalling end-of-output.
    });
}
