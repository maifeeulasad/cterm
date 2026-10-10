//! The `cterm-web` binary: serves the webview over the SQLite record store.
//!
//! Configuration via environment:
//!   * `CTERM_WEB_ADDR` — listen address (default `127.0.0.1:8088`).
//!   * `CTERM_DB`       — SQLite database path (default `cterm.db`).

use std::net::SocketAddr;
use std::sync::Arc;

use cterm_core::RecordStore;
use cterm_storage::SqliteStore;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr: SocketAddr = std::env::var("CTERM_WEB_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8088".to_owned())
        .parse()?;
    let db = std::env::var("CTERM_DB").unwrap_or_else(|_| "cterm.db".to_owned());

    let store: Arc<dyn RecordStore> = Arc::new(SqliteStore::open(&db)?);
    let app = cterm_web::router(store);

    eprintln!("cterm-web on http://{addr} (db: {db})");
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await?;
    Ok(())
}
