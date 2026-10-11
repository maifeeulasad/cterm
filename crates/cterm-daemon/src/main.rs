//! The `cterm-daemon` binary: serves the gRPC Terminal API over TCP, backed by
//! a SQLite store.
//!
//! Configuration via environment:
//!   * `CTERM_ADDR` — listen address (default `127.0.0.1:50051`).
//!   * `CTERM_DB`   — SQLite database path (default `cterm.db`).

use std::net::SocketAddr;
use std::sync::Arc;

use cterm_daemon::{terminal_server, Daemon};
use cterm_storage::SqliteStore;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr: SocketAddr = std::env::var("CTERM_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:50051".to_owned())
        .parse()?;
    let db = std::env::var("CTERM_DB").unwrap_or_else(|_| "cterm.db".to_owned());

    let store = Arc::new(SqliteStore::open(&db)?);
    let daemon = Daemon::new(store);

    // Live shim IPC over a Unix socket (Unix only).
    #[cfg(unix)]
    {
        if let Some(sock) = cterm_ipc::default_socket_path() {
            let ipc_daemon = daemon.clone();
            eprintln!("cterm-daemon ipc socket: {}", sock.display());
            tokio::spawn(async move {
                if let Err(e) = ipc_daemon.serve_ipc(&sock).await {
                    eprintln!("cterm-daemon ipc error: {e}");
                }
            });
        }
    }

    eprintln!("cterm-daemon listening on {addr} (db: {db})");
    tonic::transport::Server::builder()
        .add_service(terminal_server(daemon))
        .serve(addr)
        .await?;

    Ok(())
}
