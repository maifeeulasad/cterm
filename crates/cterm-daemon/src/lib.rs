//! # cterm-daemon
//!
//! The host-side daemon. [`Daemon`] owns the record store and the registry of
//! live runs; [`TerminalService`] exposes it as the gRPC `Terminal` service that
//! agents connect to. The agent drives executions through this service and never
//! opens its own connection to the host.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod auth;
pub mod daemon;
pub mod handle;
pub mod ipc;
pub mod service;

pub use auth::{verifier_from_env, AuthInterceptor, TokenVerifier, Verifier};
pub use daemon::{Daemon, Started};
pub use handle::ExecutionHandle;
pub use service::TerminalService;

use cterm_core::RecordStore;
use cterm_proto::v1::terminal_server::TerminalServer;
use tonic::codegen::InterceptedService;

/// Builds an *unauthenticated* gRPC server service for a daemon.
///
/// Preserves pre-0.6.0 behavior; new call sites should prefer
/// [`terminal_server_with_auth`] so an operator can opt into
/// [`TokenVerifier`] without touching service wiring.
pub fn terminal_server<S>(daemon: Daemon<S>) -> TerminalServer<TerminalService<S>>
where
    S: RecordStore + Send + Sync + 'static,
{
    TerminalServer::new(TerminalService::new(daemon))
}

/// Builds the gRPC server service for a daemon, gated by `verifier`.
///
/// The returned service is ready to hand to
/// `tonic::transport::Server::add_service`. Every RPC on it passes through
/// [`AuthInterceptor`] first, so there is no code path from the socket to
/// a [`TerminalService`] method that skips the check.
pub fn terminal_server_with_auth<S>(
    daemon: Daemon<S>,
    verifier: Verifier,
) -> InterceptedService<TerminalServer<TerminalService<S>>, AuthInterceptor>
where
    S: RecordStore + Send + Sync + 'static,
{
    let server = TerminalServer::new(TerminalService::new(daemon));
    InterceptedService::new(server, AuthInterceptor::new(verifier))
}
