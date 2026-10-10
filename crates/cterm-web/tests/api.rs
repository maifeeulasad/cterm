//! Exercises the web JSON API against a seeded in-memory store, driving the
//! router directly with `oneshot` (no socket).

use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use cterm_core::{
    Command, ExecutionRecord, ExecutionStatus, Host, RecordStore, Session, StreamChunk, StreamKind,
};
use cterm_storage::MemoryStore;
use tower::ServiceExt;

async fn get_json(app: &axum::Router, uri: &str) -> (u16, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let bytes = hyper::body::to_bytes(resp.into_body()).await.unwrap();
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json)
}

#[tokio::test]
async fn api_lists_sessions_executions_and_transcript() {
    let store = Arc::new(MemoryStore::new());

    // Seed a session with one completed execution and some output.
    let session = Session::open(Host::local(), "bash");
    store.put_session(&session).await.unwrap();
    let mut rec = ExecutionRecord::open(session.id, Command::new("echo hi", "bash"));
    rec.transition(ExecutionStatus::Completed { code: 0 });
    store.put_execution(&rec).await.unwrap();
    store
        .append_chunk(&StreamChunk::now(rec.id, StreamKind::Stdin, 0, b"echo hi\n".to_vec()))
        .await
        .unwrap();
    store
        .append_chunk(&StreamChunk::now(rec.id, StreamKind::Stdout, 1, b"hello-web\n".to_vec()))
        .await
        .unwrap();

    let app = cterm_web::router(store);

    // Sessions.
    let (code, sessions) = get_json(&app, "/api/sessions").await;
    assert_eq!(code, 200);
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    assert_eq!(sessions[0]["shell"], "bash");

    // Executions in the session.
    let uri = format!("/api/sessions/{}/executions", session.id);
    let (code, execs) = get_json(&app, &uri).await;
    assert_eq!(code, 200);
    assert_eq!(execs.as_array().unwrap().len(), 1);
    assert_eq!(execs[0]["command"]["raw"], "echo hi");

    // Transcript interleaves stdin+stdout in seq order.
    let uri = format!("/api/executions/{}/transcript", rec.id);
    let (code, body) = get_json(&app, &uri).await;
    assert_eq!(code, 200);
    let text = body["text"].as_str().unwrap();
    assert!(text.contains("echo hi"), "transcript: {text:?}");
    assert!(text.contains("hello-web"), "transcript: {text:?}");
    assert_eq!(body["record"]["status"]["kind"], "completed");

    // Unknown execution -> 404.
    let (code, _) = get_json(&app, "/api/executions/exe-00000000-0000-0000-0000-000000000000/transcript").await;
    assert_eq!(code, 404);

    // Bad id -> 400.
    let (code, _) = get_json(&app, "/api/executions/not-a-uuid/transcript").await;
    assert_eq!(code, 400);
}
