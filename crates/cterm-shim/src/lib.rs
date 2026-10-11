//! # cterm-shim
//!
//! The shell shim. It masquerades as the shell on `PATH` (the real shell having
//! been moved aside, e.g. `zsh` → `zsh-original`), runs that real shell under a
//! [`pty::PtySession`], and streams everything it observes to the daemon.
//!
//! This crate currently provides the PTY supervision layer; the daemon wiring
//! and the installer land in later changes.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// Environment variable set on every PTY-spawned process tree to mark that a
/// cterm session is already active. The PATH shim checks it and passes through
/// to the real shell when set, preventing nested re-wrapping (infinite
/// recursion / loopback).
pub const WRAPPED_MARKER_ENV: &str = "CTERM_WRAPPED";

pub mod pty;
pub mod runner;

pub use pty::{OutputReceiver, PtySession, Spawn};
pub use runner::{start_recorded, RecordedRun, RunControl, RunSpec};
