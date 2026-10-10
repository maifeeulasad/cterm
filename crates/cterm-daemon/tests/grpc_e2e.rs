//! End-to-end gRPC tests: a real tonic server over an ephemeral TCP port with a
//! real client. Unix-only (drives /bin/bash under a PTY).
#![cfg(unix)]

use std::sync::Arc;
use std::time::Duration;

use cterm_daemon::{terminal_server, Daemon};
use cterm_proto::v1;
use cterm_proto::v1::execution_event::Event;
use cterm_proto::v1::terminal_client::TerminalClient;
use cterm_storage::MemoryStore;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tokio_stream::StreamExt;

/// Starts an in-process daemon on a random port; returns its URL.
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

fn completed_code(s: &v1::Status) -> Option<i32> {
    (s.kind == v1::status::Kind::Completed as i32).then_some(s.code)
}

#[tokio::test]
async fn run_streams_events_then_records_are_queryable() {
    let url = start_server().await;
    let mut client = TerminalClient::connect(url).await.unwrap();

    let mut stream = client
        .run(v1::RunRequest {
            command: "printf GRPCOUT; exit 5".into(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    let mut started = None;
    let mut out = Vec::new();
    let mut status = None;
    while let Some(ev) = stream.next().await {
        match ev.unwrap().event.unwrap() {
            Event::Started(r) => started = Some(r),
            Event::Chunk(c) => out.extend_from_slice(&c.data),
            Event::Status(s) => status = Some(s),
        }
    }

    let started = started.expect("Started event");
    assert!(
        String::from_utf8_lossy(&out).contains("GRPCOUT"),
        "stdout was {:?}",
        String::from_utf8_lossy(&out)
    );
    assert_eq!(completed_code(&status.expect("Status event")), Some(5));

    // The record is persisted and queryable afterwards.
    let got = client
        .get_execution(v1::GetExecutionRequest {
            execution: started.id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .execution
        .unwrap();
    assert_eq!(completed_code(&got.status.unwrap()), Some(5));

    let listed = client
        .list_executions(v1::ListExecutionsRequest {
            session: started.session.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(listed.executions.len(), 1);
    assert_eq!(listed.executions[0].id, started.id);

    let sessions = client
        .list_sessions(v1::ListSessionsRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(sessions.sessions.len(), 1);
}

#[tokio::test]
async fn send_input_drives_an_interactive_shell() {
    let url = start_server().await;
    let mut client = TerminalClient::connect(url).await.unwrap();

    let mut stream = client
        .run(v1::RunRequest {
            command: "(interactive)".into(),
            program: "/bin/bash".into(),
            args: vec!["-i".into()],
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    // First event is Started; grab the execution id.
    let id = match stream.next().await.unwrap().unwrap().event.unwrap() {
        Event::Started(r) => r.id,
        other => panic!("expected Started, got {other:?}"),
    };

    client
        .send_input(v1::SendInputRequest {
            execution: id.clone(),
            data: b"echo val-$((6*7))\n".to_vec(),
        })
        .await
        .unwrap();
    client
        .send_input(v1::SendInputRequest {
            execution: id.clone(),
            data: b"exit\n".to_vec(),
        })
        .await
        .unwrap();

    let mut out = Vec::new();
    let mut status = None;
    while let Some(ev) = stream.next().await {
        match ev.unwrap().event.unwrap() {
            Event::Chunk(c) if c.kind == v1::StreamKind::Stdout as i32 => {
                out.extend_from_slice(&c.data)
            }
            Event::Status(s) => status = Some(s),
            _ => {}
        }
    }

    assert!(
        String::from_utf8_lossy(&out).contains("val-42"),
        "stdout was {:?}",
        String::from_utf8_lossy(&out)
    );
    assert_eq!(completed_code(&status.expect("Status")), Some(0));
}

#[tokio::test]
async fn interrupt_stops_a_running_command() {
    let url = start_server().await;
    let mut client = TerminalClient::connect(url).await.unwrap();

    let mut stream = client
        .run(v1::RunRequest {
            command: "sleep 30".into(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    let id = match stream.next().await.unwrap().unwrap().event.unwrap() {
        Event::Started(r) => r.id,
        other => panic!("expected Started, got {other:?}"),
    };

    tokio::time::sleep(Duration::from_millis(400)).await;
    client
        .interrupt(v1::InterruptRequest {
            execution: id.clone(),
        })
        .await
        .unwrap();

    let mut status = None;
    let drained = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(ev) = stream.next().await {
            if let Event::Status(s) = ev.unwrap().event.unwrap() {
                status = Some(s);
            }
        }
    })
    .await;
    assert!(drained.is_ok(), "stream did not end after interrupt");

    let s = status.expect("Status");
    assert_eq!(s.kind, v1::status::Kind::Interrupted as i32);
    assert!(s.has_signal && s.signal == 2);
}

#[tokio::test]
async fn stream_execution_replays_history() {
    let url = start_server().await;
    let mut client = TerminalClient::connect(url).await.unwrap();

    // Run to completion first.
    let mut run = client
        .run(v1::RunRequest {
            command: "printf REPLAYME".into(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let mut id = String::new();
    while let Some(ev) = run.next().await {
        if let Event::Started(r) = ev.unwrap().event.unwrap() {
            id = r.id;
        }
    }
    assert!(!id.is_empty());

    // Now replay it.
    let mut replay = client
        .stream_execution(v1::StreamExecutionRequest {
            execution: id.clone(),
            replay: true,
        })
        .await
        .unwrap()
        .into_inner();

    let mut saw_started = false;
    let mut out = Vec::new();
    let mut status = None;
    while let Some(ev) = replay.next().await {
        match ev.unwrap().event.unwrap() {
            Event::Started(_) => saw_started = true,
            Event::Chunk(c) => out.extend_from_slice(&c.data),
            Event::Status(s) => status = Some(s),
        }
    }

    assert!(saw_started);
    assert!(String::from_utf8_lossy(&out).contains("REPLAYME"));
    assert_eq!(completed_code(&status.expect("Status")), Some(0));
}
