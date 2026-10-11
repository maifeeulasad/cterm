//! End-to-end: a shim connected over IPC is driven by an agent over gRPC.
//! A fake shim (a raw Unix socket speaking the IPC protocol) registers a
//! session/execution; a gRPC client streams its live events and sends input
//! back, which must arrive at the shim. Unix-only.
#![cfg(unix)]

use std::sync::Arc;
use std::time::Duration;

use cterm_core::{Command, ExecutionRecord, ExecutionStatus, Host, Session, StreamChunk, StreamKind};
use cterm_daemon::{terminal_server, Daemon};
use cterm_ipc::{read_msg, write_msg, DaemonToShim, ShimToDaemon, PROTOCOL_VERSION};
use cterm_proto::v1;
use cterm_proto::v1::execution_event::Event;
use cterm_proto::v1::terminal_client::TerminalClient;
use cterm_storage::MemoryStore;
use tokio::net::{TcpListener, UnixStream};
use tokio_stream::wrappers::TcpListenerStream;
use tokio_stream::StreamExt;

fn temp_sock() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "cterm-ipc-{}-{}.sock",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// Starts a daemon serving gRPC (random TCP port) and IPC (temp socket).
async fn start() -> (String, std::path::PathBuf) {
    let daemon = Daemon::new(Arc::new(MemoryStore::new()));
    let sock = temp_sock();

    let ipc_daemon = daemon.clone();
    let ipc_sock = sock.clone();
    tokio::spawn(async move {
        let _ = ipc_daemon.serve_ipc(&ipc_sock).await;
    });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(terminal_server(daemon))
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .unwrap();
    });

    (format!("http://{addr}"), sock)
}

async fn connect_shim(sock: &std::path::Path) -> UnixStream {
    for _ in 0..50 {
        if let Ok(s) = UnixStream::connect(sock).await {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("could not connect to ipc socket");
}

#[tokio::test]
async fn agent_streams_and_controls_a_shim_over_ipc() {
    let (url, sock) = start().await;
    let mut shim = connect_shim(&sock).await;

    // The shim registers a session and an execution.
    let session = Session::open(Host::named("prod-1", true), "bash");
    let mut record = ExecutionRecord::open(session.id, Command::new("bash -i", "bash"));
    record.transition(ExecutionStatus::Running);
    let execution = record.id;

    write_msg(&mut shim, &ShimToDaemon::Hello { version: PROTOCOL_VERSION, session: session.clone() })
        .await
        .unwrap();
    write_msg(&mut shim, &ShimToDaemon::Started { record: record.clone() })
        .await
        .unwrap();

    // Agent connects; wait until the daemon has the execution recorded.
    let mut client = TerminalClient::connect(url).await.unwrap();
    let exec_str = execution.to_string();
    let mut ready = false;
    for _ in 0..50 {
        if client
            .get_execution(v1::GetExecutionRequest { execution: exec_str.clone() })
            .await
            .is_ok()
        {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(ready, "daemon never registered the shim execution");

    // Agent opens a live stream (subscribes before we send more events).
    let mut stream = client
        .stream_execution(v1::StreamExecutionRequest { execution: exec_str.clone(), replay: false })
        .await
        .unwrap()
        .into_inner();

    // First event is the Started record (from the store).
    match stream.next().await.unwrap().unwrap().event.unwrap() {
        Event::Started(r) => assert_eq!(r.id, exec_str),
        other => panic!("expected Started, got {other:?}"),
    }

    // Shim emits output -> agent sees it live.
    write_msg(
        &mut shim,
        &ShimToDaemon::Chunk {
            chunk: StreamChunk::now(execution, StreamKind::Stdout, 0, b"live-out\n".to_vec()),
        },
    )
    .await
    .unwrap();
    let out = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Event::Chunk(c) = stream.next().await.unwrap().unwrap().event.unwrap() {
                return c.data;
            }
        }
    })
    .await
    .expect("no chunk streamed");
    assert_eq!(out, b"live-out\n");

    // Agent sends input -> it must arrive at the shim.
    client
        .send_input(v1::SendInputRequest {
            execution: exec_str.clone(),
            data: b"whoami\n".to_vec(),
        })
        .await
        .unwrap();
    let got = tokio::time::timeout(Duration::from_secs(5), read_msg::<_, DaemonToShim>(&mut shim))
        .await
        .expect("timed out waiting for input at shim")
        .unwrap()
        .unwrap();
    match got {
        DaemonToShim::Input { execution: e, data } => {
            assert_eq!(e, execution);
            assert_eq!(data, b"whoami\n");
        }
        other => panic!("expected Input, got {other:?}"),
    }

    // Agent interrupts -> shim receives Interrupt.
    client
        .interrupt(v1::InterruptRequest { execution: exec_str.clone() })
        .await
        .unwrap();
    let got = tokio::time::timeout(Duration::from_secs(5), read_msg::<_, DaemonToShim>(&mut shim))
        .await
        .expect("timed out waiting for interrupt")
        .unwrap()
        .unwrap();
    assert!(matches!(got, DaemonToShim::Interrupt { .. }));

    // Shim reports completion -> the live stream ends with that status.
    write_msg(
        &mut shim,
        &ShimToDaemon::Status { execution, status: ExecutionStatus::Completed { code: 0 } },
    )
    .await
    .unwrap();
    let final_status = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match stream.next().await {
                Some(ev) => {
                    if let Event::Status(s) = ev.unwrap().event.unwrap() {
                        return Some(s);
                    }
                }
                None => return None,
            }
        }
    })
    .await
    .expect("timed out waiting for terminal status");
    assert_eq!(final_status.unwrap().kind, v1::status::Kind::Completed as i32);

    let _ = std::fs::remove_file(&sock);
}
