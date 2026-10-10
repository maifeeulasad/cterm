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
    Command, ExecutionEvent, ExecutionStatus, Host, RecordStore, Session, StreamKind,
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

async fn record_session(db: &str, name: &str, real: &str, args: &[String]) -> CliResult<i32> {
    let store = Arc::new(SqliteStore::open(db).map_err(|e| CliError::Msg(e.to_string()))?);

    let session = Session::open(Host::local(), name);
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

    // Window resize: forward SIGWINCH to the PTY.
    let resize_control = run.control();
    let winch_task = tokio::spawn(async move {
        if let Ok(mut winch) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())
        {
            while winch.recv().await.is_some() {
                let (cols, rows) = current_size();
                let _ = resize_control.resize(rows, cols);
            }
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
