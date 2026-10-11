//! End-to-end: a shim connected over IPC is driven by an agent over gRPC.
//! A fake shim (a raw IPC connection speaking the protocol) registers a
//! session/execution; a gRPC client streams its live events and sends input
//! back, which must arrive at the shim. Runs over a Unix socket on Unix and a
//! named pipe on Windows.
#![cfg(any(unix, windows))]

use std::sync::Arc;
use std::time::Duration;

use cterm_core::{
    Command, ExecutionRecord, ExecutionStatus, Host, Session, StreamChunk, StreamKind,
};
use cterm_daemon::{terminal_server, Daemon};
use cterm_ipc::{read_msg, write_msg, DaemonToShim, ShimToDaemon, PROTOCOL_VERSION};
use cterm_proto::v1;
use cterm_proto::v1::execution_event::Event;
use cterm_proto::v1::terminal_client::TerminalClient;
use cterm_storage::MemoryStore;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tokio_stream::StreamExt;

fn temp_endpoint() -> String {
    let unique = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    #[cfg(unix)]
    {
        std::env::temp_dir()
            .join(format!("cterm-ipc-{unique}.sock"))
            .to_string_lossy()
            .into_owned()
    }
    #[cfg(windows)]
    {
        format!(r"\\.\pipe\cterm-test-{unique}")
    }
}

/// Starts a daemon serving gRPC (random TCP port) and IPC (temp endpoint).
async fn start() -> (String, String) {
    let daemon = Daemon::new(Arc::new(MemoryStore::new()));
    let endpoint = temp_endpoint();

    let ipc_daemon = daemon.clone();
    let ipc_endpoint = endpoint.clone();
    tokio::spawn(async move {
        let _ = ipc_daemon.serve_ipc(&ipc_endpoint).await;
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

    (format!("http://{addr}"), endpoint)
}

#[cfg(unix)]
async fn connect_shim(endpoint: &str) -> impl AsyncRead + AsyncWrite + Unpin {
    for _ in 0..100 {
        if let Ok(s) = tokio::net::UnixStream::connect(endpoint).await {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("could not connect to ipc endpoint");
}

#[cfg(windows)]
async fn connect_shim(endpoint: &str) -> impl AsyncRead + AsyncWrite + Unpin {
    for _ in 0..100 {
        if let Ok(s) = tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint) {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("could not connect to ipc endpoint");
}

#[tokio::test]
async fn agent_streams_and_controls_a_shim_over_ipc() {
    let (url, endpoint) = start().await;
    let shim = connect_shim(&endpoint).await;
    run_scenario(shim, url).await;
    #[cfg(unix)]
    let _ = std::fs::remove_file(&endpoint);
}

async fn run_scenario<S>(mut shim: S, url: String)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // The shim registers a session and an execution.
    let session = Session::open(Host::named("prod-1", true), "bash");
    let mut record = ExecutionRecord::open(session.id, Command::new("bash -i", "bash"));
    record.transition(ExecutionStatus::Running);
    let execution = record.id;

    write_msg(
        &mut shim,
        &ShimToDaemon::Hello {
            version: PROTOCOL_VERSION,
            session: session.clone(),
        },
    )
    .await
    .unwrap();
    write_msg(
        &mut shim,
        &ShimToDaemon::Started {
            record: record.clone(),
        },
    )
    .await
    .unwrap();

    // Agent connects; wait until the daemon has the execution recorded.
    let mut client = connect_client(&url).await;
    let exec_str = execution.to_string();
    let mut ready = false;
    for _ in 0..100 {
        if client
            .get_execution(v1::GetExecutionRequest {
                execution: exec_str.clone(),
            })
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
        .stream_execution(v1::StreamExecutionRequest {
            execution: exec_str.clone(),
            replay: false,
        })
        .await
        .unwrap()
        .into_inner();
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
    let got = tokio::time::timeout(
        Duration::from_secs(5),
        read_msg::<_, DaemonToShim>(&mut shim),
    )
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
        .interrupt(v1::InterruptRequest {
            execution: exec_str.clone(),
        })
        .await
        .unwrap();
    let got = tokio::time::timeout(
        Duration::from_secs(5),
        read_msg::<_, DaemonToShim>(&mut shim),
    )
    .await
    .expect("timed out waiting for interrupt")
    .unwrap()
    .unwrap();
    assert!(matches!(got, DaemonToShim::Interrupt { .. }));

    // Shim reports completion -> the live stream ends with that status.
    write_msg(
        &mut shim,
        &ShimToDaemon::Status {
            execution,
            status: ExecutionStatus::Completed { code: 0 },
        },
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
    assert_eq!(
        final_status.unwrap().kind,
        v1::status::Kind::Completed as i32
    );
}

async fn connect_client(url: &str) -> TerminalClient<tonic::transport::Channel> {
    for _ in 0..50 {
        if let Ok(c) = TerminalClient::connect(url.to_owned()).await {
            return c;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("could not connect gRPC client");
}
