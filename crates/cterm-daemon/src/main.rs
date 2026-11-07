//! The `cterm-daemon` binary: serves the gRPC Terminal API over TCP, backed by
//! a SQLite store.
//!
//! Configuration via environment:
//!   * `CTERM_ADDR`       — listen address (default `127.0.0.1:50051`).
//!   * `CTERM_DB`         — SQLite database path (default `cterm.db`).
//!   * `CTERM_AUTH_TOKEN` — if set, require this bearer token on every RPC.
//!   * `CTERM_AUTH=on`    — enable auth and read the token from the OS keyring
//!     (the shim/CLI stashes it there via `cterm auth`).

use std::net::SocketAddr;
use std::sync::Arc;

use cterm_daemon::{terminal_server_with_auth, verifier_from_env, Daemon};
use cterm_storage::SqliteStore;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr: SocketAddr = std::env::var("CTERM_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:50051".to_owned())
        .parse()?;
    let db = std::env::var("CTERM_DB").unwrap_or_else(|_| "cterm.db".to_owned());

    // The daemon binary intentionally doesn't link the `keyring` crate: that's
    // the CLI's job. If `CTERM_AUTH=on` without `CTERM_AUTH_TOKEN`, we fall
    // through to NoopVerifier and print a warning so operators notice.
    let (verifier, auth_enabled) = verifier_from_env(None);

    let store = Arc::new(SqliteStore::open(&db)?);
    let daemon = Daemon::new(store);

    // Live shim IPC: Unix socket on Unix, named pipe on Windows.
    if let Some(endpoint) = cterm_ipc::default_endpoint() {
        let ipc_daemon = daemon.clone();
        eprintln!("cterm-daemon ipc endpoint: {endpoint}");
        tokio::spawn(async move {
            if let Err(e) = ipc_daemon.serve_ipc(&endpoint).await {
                eprintln!("cterm-daemon ipc error: {e}");
            }
        });
    }

    let auth_banner = if auth_enabled {
        "auth=on"
    } else if std::env::var("CTERM_AUTH").is_ok() {
        "auth=off (CTERM_AUTH set but no token; set CTERM_AUTH_TOKEN or run `cterm auth enable`)"
    } else {
        "auth=off"
    };
    eprintln!("cterm-daemon listening on {addr} (db: {db}, {auth_banner})");
    tonic::transport::Server::builder()
        .add_service(terminal_server_with_auth(daemon, verifier))
        .serve(addr)
        .await?;

    Ok(())
}
