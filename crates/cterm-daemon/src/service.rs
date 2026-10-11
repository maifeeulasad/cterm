//! The gRPC adapter: implements the generated `Terminal` service over [`Daemon`].

use std::pin::Pin;

use cterm_core::{Error, ExecutionEvent, ExecutionId, RecordStore, SessionId};
use cterm_proto::convert;
use cterm_proto::v1;
use futures_core::Stream;
use tokio::sync::broadcast::error::RecvError;
use tonic::{Request, Response, Status};

use crate::daemon::Daemon;

/// A server-streamed sequence of execution events.
type EventStream =
    Pin<Box<dyn Stream<Item = std::result::Result<v1::ExecutionEvent, Status>> + Send>>;

/// gRPC front end for the daemon.
pub struct TerminalService<S: RecordStore> {
    daemon: Daemon<S>,
}

impl<S: RecordStore + Send + Sync + 'static> TerminalService<S> {
    /// Wraps a daemon.
    pub fn new(daemon: Daemon<S>) -> Self {
        Self { daemon }
    }
}

fn to_status(e: Error) -> Status {
    match e {
        Error::NotFound(m) => Status::not_found(m),
        Error::Forbidden(m) => Status::permission_denied(m),
        Error::Other(m) => Status::invalid_argument(m),
        other => Status::internal(other.to_string()),
    }
}

fn parse_execution(s: &str) -> std::result::Result<ExecutionId, Status> {
    s.parse::<ExecutionId>()
        .map_err(|e| Status::invalid_argument(format!("invalid execution id {s:?}: {e}")))
}

fn parse_session(s: &str) -> std::result::Result<SessionId, Status> {
    s.parse::<SessionId>()
        .map_err(|e| Status::invalid_argument(format!("invalid session id {s:?}: {e}")))
}

/// Converts a live broadcast receiver into a response stream that ends after
/// the terminal `Status` event. Lag is skipped (clients resync by seq);
/// closure ends the stream.
fn live_stream(mut rx: tokio::sync::broadcast::Receiver<ExecutionEvent>) -> EventStream {
    Box::pin(async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let terminal = matches!(event, ExecutionEvent::Status(_));
                    yield Ok(convert::event(&event));
                    if terminal {
                        break;
                    }
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    })
}

#[tonic::async_trait]
impl<S: RecordStore + Send + Sync + 'static> v1::terminal_server::Terminal for TerminalService<S> {
    type RunStream = EventStream;
    type StreamExecutionStream = EventStream;

    async fn run(
        &self,
        request: Request<v1::RunRequest>,
    ) -> std::result::Result<Response<Self::RunStream>, Status> {
        let started = self
            .daemon
            .run(request.into_inner())
            .await
            .map_err(to_status)?;
        Ok(Response::new(live_stream(started.events)))
    }

    async fn stream_execution(
        &self,
        request: Request<v1::StreamExecutionRequest>,
    ) -> std::result::Result<Response<Self::StreamExecutionStream>, Status> {
        let req = request.into_inner();
        let execution = parse_execution(&req.execution)?;

        if req.replay {
            // Historical snapshot: everything the store holds, in order, ending
            // with the recorded status (which is `Running` if not yet finished).
            let (record, chunks) = self
                .daemon
                .history(execution)
                .await
                .map_err(to_status)?
                .ok_or_else(|| Status::not_found(format!("unknown execution {execution}")))?;
            let stream = async_stream::stream! {
                yield Ok(convert::event(&ExecutionEvent::Started(record.clone())));
                for chunk in chunks {
                    yield Ok(convert::event(&ExecutionEvent::Chunk(chunk)));
                }
                yield Ok(convert::event(&ExecutionEvent::Status(record.status)));
            };
            return Ok(Response::new(Box::pin(stream)));
        }

        // Live tail: the current record, then live events to completion if it is
        // still running, otherwise its final status.
        let record = self
            .daemon
            .get_execution(execution)
            .await
            .map_err(to_status)?
            .ok_or_else(|| Status::not_found(format!("unknown execution {execution}")))?;
        let live = self.daemon.subscribe(execution);
        let stream = async_stream::stream! {
            yield Ok(convert::event(&ExecutionEvent::Started(record.clone())));
            if let Some(mut rx) = live {
                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            let terminal = matches!(event, ExecutionEvent::Status(_));
                            yield Ok(convert::event(&event));
                            if terminal { break; }
                        }
                        Err(RecvError::Lagged(_)) => continue,
                        Err(RecvError::Closed) => break,
                    }
                }
            } else {
                yield Ok(convert::event(&ExecutionEvent::Status(record.status)));
            }
        };
        Ok(Response::new(Box::pin(stream)))
    }

    async fn send_input(
        &self,
        request: Request<v1::SendInputRequest>,
    ) -> std::result::Result<Response<v1::SendInputResponse>, Status> {
        let req = request.into_inner();
        let execution = parse_execution(&req.execution)?;
        self.daemon
            .send_input(execution, req.data)
            .await
            .map_err(to_status)?;
        Ok(Response::new(v1::SendInputResponse {}))
    }

    async fn interrupt(
        &self,
        request: Request<v1::InterruptRequest>,
    ) -> std::result::Result<Response<v1::InterruptResponse>, Status> {
        let req = request.into_inner();
        let execution = parse_execution(&req.execution)?;
        self.daemon.interrupt(execution).await.map_err(to_status)?;
        Ok(Response::new(v1::InterruptResponse {}))
    }

    async fn list_sessions(
        &self,
        _request: Request<v1::ListSessionsRequest>,
    ) -> std::result::Result<Response<v1::ListSessionsResponse>, Status> {
        let sessions = self.daemon.list_sessions().await.map_err(to_status)?;
        Ok(Response::new(v1::ListSessionsResponse {
            sessions: sessions.iter().map(convert::session).collect(),
        }))
    }

    async fn list_executions(
        &self,
        request: Request<v1::ListExecutionsRequest>,
    ) -> std::result::Result<Response<v1::ListExecutionsResponse>, Status> {
        let session = parse_session(&request.into_inner().session)?;
        let executions = self
            .daemon
            .list_executions(session)
            .await
            .map_err(to_status)?;
        Ok(Response::new(v1::ListExecutionsResponse {
            executions: executions.iter().map(convert::record).collect(),
        }))
    }

    async fn get_execution(
        &self,
        request: Request<v1::GetExecutionRequest>,
    ) -> std::result::Result<Response<v1::GetExecutionResponse>, Status> {
        let execution = parse_execution(&request.into_inner().execution)?;
        let record = self
            .daemon
            .get_execution(execution)
            .await
            .map_err(to_status)?
            .ok_or_else(|| Status::not_found(format!("unknown execution {execution}")))?;
        Ok(Response::new(v1::GetExecutionResponse {
            execution: Some(convert::record(&record)),
        }))
    }
}
