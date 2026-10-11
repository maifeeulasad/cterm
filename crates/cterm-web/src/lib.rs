//! # cterm-web
//!
//! A basic webview over recorded executions. It reads the same
//! [`RecordStore`](cterm_core::RecordStore) the daemon and shims write to, and
//! exposes a small JSON API plus a static single-page UI:
//!
//!   * `GET /`                                      — the UI
//!   * `GET /api/sessions`                          — all sessions
//!   * `GET /api/sessions/:id/executions`           — executions in a session
//!   * `GET /api/executions/:id`                    — one execution record
//!   * `GET /api/executions/:id/transcript`         — record + reconstructed text

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use cterm_core::{ExecutionId, ExecutionRecord, RecordStore, Session, SessionId};
use serde::Serialize;

/// Shared store handle used by all handlers.
type Store = Arc<dyn RecordStore>;

/// Builds the web application router backed by `store`.
pub fn router(store: Store) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/:id/executions", get(session_executions))
        .route("/api/executions/:id", get(get_execution))
        .route("/api/executions/:id/transcript", get(transcript))
        .with_state(store)
}

/// An API error mapped to an HTTP response.
struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, self.1).into_response()
    }
}

impl From<cterm_core::Error> for ApiError {
    fn from(e: cterm_core::Error) -> Self {
        let code = match e {
            cterm_core::Error::NotFound(_) => StatusCode::NOT_FOUND,
            cterm_core::Error::Forbidden(_) => StatusCode::FORBIDDEN,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError(code, e.to_string())
    }
}

fn parse_session(s: &str) -> Result<SessionId, ApiError> {
    s.parse::<SessionId>()
        .map_err(|e| ApiError(StatusCode::BAD_REQUEST, format!("bad session id: {e}")))
}

fn parse_execution(s: &str) -> Result<ExecutionId, ApiError> {
    s.parse::<ExecutionId>()
        .map_err(|e| ApiError(StatusCode::BAD_REQUEST, format!("bad execution id: {e}")))
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../web/index.html"))
}

async fn list_sessions(State(store): State<Store>) -> Result<Json<Vec<Session>>, ApiError> {
    Ok(Json(store.list_sessions().await?))
}

async fn session_executions(
    State(store): State<Store>,
    Path(id): Path<String>,
) -> Result<Json<Vec<ExecutionRecord>>, ApiError> {
    let session = parse_session(&id)?;
    Ok(Json(store.list_executions(session).await?))
}

async fn get_execution(
    State(store): State<Store>,
    Path(id): Path<String>,
) -> Result<Json<ExecutionRecord>, ApiError> {
    let execution = parse_execution(&id)?;
    store
        .get_execution(execution)
        .await?
        .map(Json)
        .ok_or_else(|| {
            ApiError(
                StatusCode::NOT_FOUND,
                format!("unknown execution {execution}"),
            )
        })
}

/// A record plus its reconstructed text transcript.
#[derive(Serialize)]
struct TranscriptResponse {
    record: ExecutionRecord,
    /// Input and output interleaved in capture (`seq`) order, lossily decoded.
    text: String,
}

async fn transcript(
    State(store): State<Store>,
    Path(id): Path<String>,
) -> Result<Json<TranscriptResponse>, ApiError> {
    let execution = parse_execution(&id)?;
    let record = store.get_execution(execution).await?.ok_or_else(|| {
        ApiError(
            StatusCode::NOT_FOUND,
            format!("unknown execution {execution}"),
        )
    })?;
    // load_chunks returns seq order, which interleaves stdin and stdout as they
    // actually occurred.
    let chunks = store.load_chunks(execution).await?;
    let mut text = String::new();
    for chunk in chunks {
        text.push_str(&String::from_utf8_lossy(&chunk.data));
    }
    Ok(Json(TranscriptResponse { record, text }))
}
