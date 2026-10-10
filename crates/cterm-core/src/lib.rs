//! # cterm-core
//!
//! The domain model and ports for **cterm**, a terminal integration/shim tool.
//!
//! This crate is the stable center of the system. It owns the vocabulary —
//! sessions, executions, stream chunks, status — and the abstract ports the
//! outer crates implement. It is runtime-agnostic and holds no I/O: it never
//! opens a socket, a database, or a PTY. That lives in the adapters.
//!
//! ## The shape of the system
//!
//! ```text
//!   real shell (zsh/bash)
//!        ▲  PTY
//!        │
//!   cterm-shim ──IPC──▶ cterm-daemon ──persist──▶ RecordStore (SQLite)
//!   (masquerades as                 │
//!    the shell)                     └──gRPC(stream)──▶ agents (Claude), webview
//! ```
//!
//! An agent never SSHes into a host itself. A human opens the session; the
//! shim on that host reports to the daemon; the agent observes and (where
//! policy allows) drives the session through the daemon's gRPC surface. That
//! is the "hybrid" path: the human holds the credentials, the agent gets a
//! governed channel.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod command;
pub mod error;
pub mod event;
pub mod ids;
pub mod ports;
pub mod record;
pub(crate) mod serde_bytes_base64;
pub mod session;
pub mod status;
pub mod stream;

// Re-export the common surface so downstream crates can `use cterm_core::*`.
pub use command::{Command, EnvVar};
pub use error::{Error, Result};
pub use event::ExecutionEvent;
pub use ids::{ExecutionId, SessionId};
pub use ports::{RecordStore, TerminalControl};
pub use record::ExecutionRecord;
pub use session::{Host, Session};
pub use status::{ExecutionStatus, SessionStatus};
pub use stream::{StreamChunk, StreamKind};
