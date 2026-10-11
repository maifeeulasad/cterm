//! Shell-wrapper mode: what the binary does when invoked as `bash`/`zsh` from
//! the shim directory.
//!
//! It runs the *real* shell under a PTY, bridges the user's terminal to it
//! (raw-mode passthrough of input, live output, window-resize), and records the
//! whole session to the shared store — which the daemon reads for queries and
//! replay.
//!
//! Robustness rule: cterm must never prevent the shell from starting. If any
//! setup step fails (no config, store won't open, not wrapped), we fall back to
//! simply `exec`-ing the real shell with no recording.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cterm_core::{
    Command, ExecutionEvent, ExecutionStatus, Host, RecordStore, Session, SessionStatus, StreamKind,
};
use cterm_shim::{start_recorded, RunSpec};
use cterm_storage::SqliteStore;

use crate::config;
use crate::commands;
use crate::error::{CliError, CliResult};
use crate::layout::Layout;

/// Runs as the wrapper for shell `name` with `args`, returning a process exit
/// code. On any setup failure it falls back to `exec`-ing the real shell.
pub fn run(name: &str, args: &[String]) -> i32 {
    match try_record(name, args) {
        Ok(code) => code,
        Err(_setup_failed) => match resolve_real_shell(name) {
            Some(path) => exec_real(&path, args),
            None => {
                eprintln!("cterm: could not locate the real '{name}' shell to run");
                127
            }
        },
    }
}

/// Attempts the full recording path. Returns `Err` only for *setup* failures
/// (before the shell starts), so `run` can safely fall back.
fn try_record(name: &str, args: &[String]) -> CliResult<i32> {
    let layout = Layout::default_home()?;
    let cfg = config::load(&layout)?;
    let real = cfg
        .shells
        .get(name)
        .cloned()
        .ok_or_else(|| CliError::Msg(format!("'{name}' is not a wrapped shell")))?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(record_session(&cfg.db, name, &real, args))
}

/// Records the session live to the daemon over IPC if it is reachable;
/// otherwise records straight to the local SQLite database.
async fn record_session(db: &str, name: &str, real: &str, args: &[String]) -> CliResult<i32> {
    #[cfg(unix)]
    {
        if let Some(code) = try_ipc_session(name, real, args).await? {
            return Ok(code);
        }
    }
    record_session_db(db, name, real, args).await
}

/// Runs the session under a PTY, forwarding everything live to the daemon over
/// the IPC socket and applying control (input/interrupt/resize) it sends back.
/// Returns `Ok(None)` if the daemon socket is not reachable (caller falls back
/// to local recording).
#[cfg(unix)]
async fn try_ipc_session(name: &str, real: &str, args: &[String]) -> CliResult<Option<i32>> {
    use cterm_ipc::{read_msg, write_msg, DaemonToShim, ShimToDaemon, PROTOCOL_VERSION};
    use cterm_storage::NullStore;
    use tokio::io::AsyncWriteExt;

    let Some(sock) = cterm_ipc::default_socket_path() else {
        return Ok(None);
    };
    let stream = match tokio::net::UnixStream::connect(&sock).await {
        Ok(s) => s,
        Err(_) => return Ok(None), // daemon not running -> fall back to DB
    };
    let (mut read_half, mut write_half) = stream.into_split();

    // The daemon persists; the shim keeps no local copy.
    let store = std::sync::Arc::new(NullStore::new());
    let session = Session::open(Host::local(), name);

    let (cols, rows) = current_size();
    let mut spec = RunSpec::new(real.to_owned(), args.to_vec());
    spec.rows = rows;
    spec.cols = cols;
    let raw = format!("{name} {}", args.join(" "));
    let command = Command::new(raw.trim().to_owned(), name);

    let mut run = start_recorded(store, session.id, command, spec)
        .await
        .map_err(|e| CliError::Msg(e.to_string()))?;
    let execution = run.execution();
    let mut events = run.take_primary().expect("primary on a fresh run");

    // Register the session with the daemon before streaming events.
    write_msg(
        &mut write_half,
        &ShimToDaemon::Hello {
            version: PROTOCOL_VERSION,
            session: session.clone(),
        },
    )
    .await
    .map_err(|e| CliError::Msg(e.to_string()))?;

    // Outbound frames to the daemon go through one writer task.
    let (tx, mut tx_rx) = tokio::sync::mpsc::unbounded_channel::<ShimToDaemon>();
    let writer = tokio::spawn(async move {
        while let Some(msg) = tx_rx.recv().await {
            if write_msg(&mut write_half, &msg).await.is_err() {
                break;
            }
        }
    });

    let _raw_guard = RawModeGuard::enable();

    // Local stdin (the human) -> the PTY.
    let (in_tx, mut in_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut stdin = std::io::stdin();
        let mut buf = [0u8; 4096];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if in_tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    let in_control = run.control();
    let in_task = tokio::spawn(async move {
        while let Some(data) = in_rx.recv().await {
            let _ = in_control.send_input(data).await;
        }
    });

    // Daemon -> shim control (agent input / interrupt / resize).
    let ctrl = run.control();
    let ctrl_task = tokio::spawn(async move {
        loop {
            match read_msg::<_, DaemonToShim>(&mut read_half).await {
                Ok(Some(DaemonToShim::Input { data, .. })) => {
                    let _ = ctrl.send_input(data).await;
                }
                Ok(Some(DaemonToShim::Interrupt { .. })) => {
                    let _ = ctrl.interrupt().await;
                }
                Ok(Some(DaemonToShim::Resize { rows, cols, .. })) => {
                    let _ = ctrl.resize(rows, cols);
                }
                Ok(None) | Err(_) => break,
            }
        }
    });

    // SIGWINCH -> resize the PTY.
    let resize_ctrl = run.control();
    let winch_task = tokio::spawn(async move {
        if let Ok(mut winch) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())
        {
            while winch.recv().await.is_some() {
                let (cols, rows) = current_size();
                let _ = resize_ctrl.resize(rows, cols);
            }
        }
    });

    // Consume events: write output to the human, forward everything to the daemon.
    let mut stdout = tokio::io::stdout();
    let status = loop {
        match events.recv().await {
            Ok(ExecutionEvent::Started(record)) => {
                let _ = tx.send(ShimToDaemon::Started { record });
            }
            Ok(ExecutionEvent::Chunk(chunk)) => {
                if chunk.kind == StreamKind::Stdout {
                    let _ = stdout.write_all(&chunk.data).await;
                    let _ = stdout.flush().await;
                }
                let _ = tx.send(ShimToDaemon::Chunk { chunk });
            }
            Ok(ExecutionEvent::Status(s)) => {
                let _ = tx.send(ShimToDaemon::Status { execution, status: s });
                break s;
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break ExecutionStatus::Failed,
        }
    };

    let _ = run.done.await;
    let _ = tx.send(ShimToDaemon::SessionClosed {
        session: session.id,
        status: SessionStatus::Closed,
    });

    in_task.abort();
    ctrl_task.abort();
    winch_task.abort();
    drop(tx);
    let _ = writer.await;

    Ok(Some(exit_code(status)))
}

async fn record_session_db(db: &str, name: &str, real: &str, args: &[String]) -> CliResult<i32> {
    let store = Arc::new(SqliteStore::open(db).map_err(|e| CliError::Msg(e.to_string()))?);

    let mut session = Session::open(Host::local(), name);
    store
        .put_session(&session)
        .await
        .map_err(|e| CliError::Msg(e.to_string()))?;

    let (cols, rows) = current_size();
    let mut spec = RunSpec::new(real.to_owned(), args.to_vec());
    spec.rows = rows;
    spec.cols = cols;

    let raw = format!("{name} {}", args.join(" "));
    let command = Command::new(raw.trim().to_owned(), name);

    let mut run = start_recorded(store.clone(), session.id, command, spec)
        .await
        .map_err(|e| CliError::Msg(e.to_string()))?;
    let mut events = run.take_primary().expect("primary on a fresh run");

    // Put the user's terminal in raw mode so keystrokes pass through verbatim;
    // restored when this guard drops. A no-op when stdin is not a tty.
    let _raw_guard = RawModeGuard::enable();

    // Output: live PTY output -> our stdout.
    let out_task = tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        let mut stdout = tokio::io::stdout();
        loop {
            match events.recv().await {
                Ok(ExecutionEvent::Chunk(c)) if c.kind == StreamKind::Stdout => {
                    let _ = stdout.write_all(&c.data).await;
                    let _ = stdout.flush().await;
                }
                Ok(ExecutionEvent::Status(_)) => break,
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Input: our stdin -> PTY (recorded as Stdin by send_input).
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut stdin = std::io::stdin();
        let mut buf = [0u8; 4096];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    let in_control = run.control();
    let in_task = tokio::spawn(async move {
        while let Some(data) = rx.recv().await {
            let _ = in_control.send_input(data).await;
        }
    });

    // Window resize: forward SIGWINCH to the PTY (Unix). Windows delivers
    // resize via console events, not signals; handled when that backend lands.
    let resize_control = run.control();
    let winch_task = tokio::spawn(async move {
        #[cfg(unix)]
        {
            if let Ok(mut winch) =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())
            {
                while winch.recv().await.is_some() {
                    let (cols, rows) = current_size();
                    let _ = resize_control.resize(rows, cols);
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = &resize_control;
            std::future::pending::<()>().await;
        }
    });

    // Wait for the shell to exit, then let output drain and stop the helpers.
    let status = run
        .done
        .await
        .map_err(|e| CliError::Msg(format!("wait join failed: {e}")))?
        .map_err(|e| CliError::Msg(e.to_string()))?;
    let _ = out_task.await;
    in_task.abort();
    winch_task.abort();

    // The shell has exited, so the session is over: mark it closed.
    session.close(SessionStatus::Closed);
    let _ = store.put_session(&session).await;

    Ok(exit_code(status))
}

fn exit_code(status: ExecutionStatus) -> i32 {
    match status {
        ExecutionStatus::Completed { code } => code,
        ExecutionStatus::Interrupted { .. } => 130, // 128 + SIGINT
        ExecutionStatus::Failed => 1,
        _ => 0,
    }
}

/// Current terminal size as `(cols, rows)`, defaulting to 80x24.
fn current_size() -> (u16, u16) {
    #[cfg(unix)]
    {
        if let Some((terminal_size::Width(w), terminal_size::Height(h))) =
            terminal_size::terminal_size()
        {
            return (w, h);
        }
    }
    (80, 24)
}

/// Finds the real shell to exec: prefer the path recorded at install time,
/// then fall back to a PATH search that excludes our shim directory.
pub fn resolve_real_shell(name: &str) -> Option<PathBuf> {
    if let Ok(layout) = Layout::default_home() {
        if let Ok(cfg) = config::load(&layout) {
            if let Some(real) = cfg.shells.get(name) {
                return Some(PathBuf::from(real));
            }
        }
        if let Some(real) = commands::find_real_shell(name, &layout.bin_dir()) {
            return Some(real);
        }
    }
    commands::find_real_shell(name, Path::new("/nonexistent"))
}

#[cfg(unix)]
fn exec_real(path: &Path, args: &[String]) -> i32 {
    use std::os::unix::process::CommandExt;
    let err = std::process::Command::new(path).args(args).exec();
    eprintln!("cterm: failed to exec {}: {err}", path.display());
    126
}

#[cfg(not(unix))]
fn exec_real(path: &Path, args: &[String]) -> i32 {
    match std::process::Command::new(path).args(args).status() {
        Ok(status) => status.code().unwrap_or(0),
        Err(err) => {
            eprintln!("cterm: failed to run {}: {err}", path.display());
            126
        }
    }
}

/// Puts stdin into raw mode for the wrapper's lifetime (Unix tty only).
#[cfg(unix)]
struct RawModeGuard {
    original: nix::sys::termios::Termios,
}

#[cfg(unix)]
impl RawModeGuard {
    fn enable() -> Option<Self> {
        use nix::sys::termios::{cfmakeraw, tcgetattr, tcsetattr, SetArg};
        use std::os::fd::AsFd;

        let stdin = std::io::stdin();
        let original = tcgetattr(stdin.as_fd()).ok()?; // Err if not a tty.
        let mut raw = original.clone();
        cfmakeraw(&mut raw);
        tcsetattr(stdin.as_fd(), SetArg::TCSANOW, &raw).ok()?;
        Some(Self { original })
    }
}

#[cfg(unix)]
impl Drop for RawModeGuard {
    fn drop(&mut self) {
        use nix::sys::termios::{tcsetattr, SetArg};
        use std::os::fd::AsFd;
        let stdin = std::io::stdin();
        let _ = tcsetattr(stdin.as_fd(), SetArg::TCSANOW, &self.original);
    }
}

#[cfg(not(unix))]
struct RawModeGuard;

#[cfg(not(unix))]
impl RawModeGuard {
    fn enable() -> Option<Self> {
        None
    }
}
