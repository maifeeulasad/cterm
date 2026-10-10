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

pub mod pty;

pub use pty::{OutputReceiver, PtySession, Spawn};
