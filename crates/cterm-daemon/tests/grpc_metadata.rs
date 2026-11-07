//! End-to-end: a client that sets title + labels on `RunRequest` gets them
//! back on `ListSessions`. Validates both wire extension and daemon plumbing.
#![cfg(unix)]

use std::sync::Arc;

use cterm_daemon::{terminal_server, Daemon};
use cterm_proto::v1;
use cterm_proto::v1::terminal_client::TerminalClient;
use cterm_storage::MemoryStore;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tokio_stream::StreamExt;

async fn start_server() -> String {
    let daemon = Daemon::new(Arc::new(MemoryStore::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(terminal_server(daemon))
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn opens_session_with_title_and_labels_then_lists_them_back() {
    let url = start_server().await;
    let mut client = TerminalClient::connect(url).await.unwrap();

    let labels = [("env", "staging"), ("ticket", "ABC-123")]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();

    // Opening a new session (session == "") with metadata attached.
    let mut stream = client
        .run(v1::RunRequest {
            command: "true".into(),
            title: "deploy webapp".into(),
            labels,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    // Drain the stream to completion so the daemon finishes the session.
    while stream.next().await.transpose().unwrap().is_some() {}

    let sessions = client
        .list_sessions(v1::ListSessionsRequest {})
        .await
        .unwrap()
        .into_inner()
        .sessions;

    assert_eq!(sessions.len(), 1);
    let s = &sessions[0];
    assert_eq!(s.title, "deploy webapp");
    assert_eq!(s.labels.get("env").map(String::as_str), Some("staging"));
    assert_eq!(s.labels.get("ticket").map(String::as_str), Some("ABC-123"));
}

#[tokio::test]
async fn rejects_invalid_title_with_invalid_argument() {
    let url = start_server().await;
    let mut client = TerminalClient::connect(url).await.unwrap();

    let err = client
        .run(v1::RunRequest {
            command: "true".into(),
            title: "line\nbreak".into(), // control char rejected
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument, "got: {err:?}");
}
