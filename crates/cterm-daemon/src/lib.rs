//! # cterm-daemon
//!
//! The host-side daemon. [`Daemon`] owns the record store and the registry of
//! live runs; [`TerminalService`] exposes it as the gRPC `Terminal` service that
//! agents connect to. The agent drives executions through this service and never
//! opens its own connection to the host.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod daemon;
pub mod service;

pub use daemon::{Daemon, Started};
pub use service::TerminalService;

use cterm_core::RecordStore;
use cterm_proto::TerminalServer;

/// Builds the gRPC server service for a daemon, ready to hand to
/// `tonic::transport::Server::add_service`.
pub fn terminal_server<S>(daemon: Daemon<S>) -> TerminalServer<TerminalService<S>>
where
    S: RecordStore + Send + Sync + 'static,
{
    TerminalServer::new(TerminalService::new(daemon))
}
