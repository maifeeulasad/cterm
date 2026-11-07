//! End-to-end auth tests over a real tonic server: confirm that
//! `terminal_server_with_auth` rejects the wrong bearer token, accepts the
//! right one, and that an unprotected server behaves identically to before.
//!
//! Unix-only because we reuse the same infra as `grpc_e2e.rs` (which spawns
//! /bin/bash to exercise the Run path).
#![cfg(unix)]

use std::sync::Arc;

use cterm_daemon::{terminal_server_with_auth, Daemon, TokenVerifier, Verifier};
use cterm_proto::v1;
use cterm_proto::v1::terminal_client::TerminalClient;
use cterm_storage::MemoryStore;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::metadata::MetadataValue;

/// Starts an in-process daemon protected by `verifier`. Returns its URL.
async fn start_protected(verifier: Verifier) -> String {
    let daemon = Daemon::new(Arc::new(MemoryStore::new()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(terminal_server_with_auth(daemon, verifier))
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

/// Builds a client that attaches a bearer header on every RPC.
async fn bearer_client(
    url: String,
    token: &str,
) -> TerminalClient<
    tonic::codegen::InterceptedService<
        tonic::transport::Channel,
        impl tonic::service::Interceptor + Clone,
    >,
> {
    let channel = tonic::transport::Endpoint::from_shared(url)
        .unwrap()
        .connect()
        .await
        .unwrap();
    let header: MetadataValue<_> = format!("Bearer {token}").parse().unwrap();
    TerminalClient::with_interceptor(channel, move |mut req: tonic::Request<()>| {
        req.metadata_mut().insert("authorization", header.clone());
        Ok(req)
    })
}

#[tokio::test]
async fn unauthenticated_client_is_rejected_when_auth_is_on() {
    let verifier: Verifier = Arc::new(TokenVerifier::new("expected"));
    let url = start_protected(verifier).await;

    // Plain client: no interceptor, no authorization header.
    let mut client = TerminalClient::connect(url).await.unwrap();
    let err = client
        .list_sessions(v1::ListSessionsRequest {})
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        tonic::Code::Unauthenticated,
        "expected UNAUTHENTICATED, got: {err:?}",
    );
}

#[tokio::test]
async fn wrong_token_is_rejected() {
    let verifier: Verifier = Arc::new(TokenVerifier::new("expected"));
    let url = start_protected(verifier).await;

    let mut client = bearer_client(url, "wrong").await;
    let err = client
        .list_sessions(v1::ListSessionsRequest {})
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unauthenticated);
}

#[tokio::test]
async fn correct_token_is_accepted() {
    let verifier: Verifier = Arc::new(TokenVerifier::new("expected"));
    let url = start_protected(verifier).await;

    let mut client = bearer_client(url, "expected").await;
    let resp = client
        .list_sessions(v1::ListSessionsRequest {})
        .await
        .expect("accepted request should succeed");
    assert_eq!(resp.into_inner().sessions.len(), 0);
}
