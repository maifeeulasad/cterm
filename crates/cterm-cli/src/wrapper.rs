//! Shell-wrapper mode: what the binary does when invoked as `bash`/`zsh` from
//! the shim directory.
//!
//! For now this transparently hands off to the real shell (recording is added
//! in the next step). Robustness rule: if anything about cterm is unavailable,
//! the wrapper must still start the user's shell — a broken daemon or config
//! can never make a login shell fail.

use std::path::{Path, PathBuf};

use crate::config;
use crate::commands;
use crate::layout::Layout;

/// Runs as the wrapper for shell `name` with `args`, returning a process exit
/// code (though on success `exec` replaces this process and never returns).
pub fn run(name: &str, args: &[String]) -> i32 {
    match resolve_real_shell(name) {
        Some(path) => exec_real(&path, args),
        None => {
            eprintln!("cterm: could not locate the real '{name}' shell to run");
            127
        }
    }
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
    // Replaces this process; only returns on failure.
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
