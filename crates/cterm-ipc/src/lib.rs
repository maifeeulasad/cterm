//! # cterm-ipc
//!
//! The internal protocol between a shim (an interactive wrapped shell, running
//! in its own process) and the daemon. The shim owns the PTY and is the
//! authority on what happened; it reports lifecycle and stream events to the
//! daemon, which persists them and relays them to agents over gRPC. The daemon
//! sends control (input, interrupt, resize) back down the same connection.
//!
//! Transport is a local stream socket (Unix domain socket). Messages are
//! length-delimited JSON: a 4-byte big-endian length followed by that many
//! bytes of JSON. Framing is generic over any `AsyncRead`/`AsyncWrite`, so it
//! is testable over an in-memory duplex.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::io;
use std::path::PathBuf;

use cterm_core::{ExecutionId, ExecutionRecord, ExecutionStatus, SessionId, SessionStatus, StreamChunk};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Maximum accepted frame size (16 MiB), a guard against a corrupt length.
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

/// A message from a shim to the daemon.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShimToDaemon {
    /// Registers the shim's session. Sent once, first.
    Hello {
        /// Protocol version for forward compatibility.
        version: u32,
        /// The session the shim opened.
        session: cterm_core::Session,
    },
    /// An execution has started.
    Started {
        /// The execution record in its running state.
        record: ExecutionRecord,
    },
    /// A chunk of input or output.
    Chunk {
        /// The stream chunk.
        chunk: StreamChunk,
    },
    /// An execution reached a terminal status.
    Status {
        /// Which execution.
        execution: ExecutionId,
        /// Its terminal status.
        status: ExecutionStatus,
    },
    /// The session has closed.
    SessionClosed {
        /// Which session.
        session: SessionId,
        /// Its terminal status.
        status: SessionStatus,
    },
}

/// A message from the daemon to a shim (control).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DaemonToShim {
    /// Write input to an execution.
    Input {
        /// Target execution.
        execution: ExecutionId,
        /// Bytes to write to its stdin.
        data: Vec<u8>,
    },
    /// Interrupt (Ctrl-C) an execution.
    Interrupt {
        /// Target execution.
        execution: ExecutionId,
    },
    /// Resize an execution's terminal.
    Resize {
        /// Target execution.
        execution: ExecutionId,
        /// New row count.
        rows: u16,
        /// New column count.
        cols: u16,
    },
}

/// The current protocol version advertised in [`ShimToDaemon::Hello`].
pub const PROTOCOL_VERSION: u32 = 1;

/// Writes one length-delimited JSON frame.
pub async fn write_msg<W, T>(w: &mut W, msg: &T) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let bytes = serde_json::to_vec(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if bytes.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message exceeds MAX_FRAME",
        ));
    }
    w.write_all(&(bytes.len() as u32).to_be_bytes()).await?;
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}

/// Reads one length-delimited JSON frame. Returns `Ok(None)` on a clean EOF at a
/// frame boundary (the peer closed the connection).
pub async fn read_msg<R, T>(r: &mut R) -> io::Result<Option<T>>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame length exceeds MAX_FRAME",
        ));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    let msg = serde_json::from_slice(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(Some(msg))
}

/// The default IPC socket path: `$CTERM_SOCK`, else `<home>/.cterm/run/daemon.sock`.
#[must_use]
pub fn default_socket_path() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("CTERM_SOCK") {
        return Some(PathBuf::from(explicit));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(
        PathBuf::from(home)
            .join(".cterm")
            .join("run")
            .join("daemon.sock"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cterm_core::{Host, Session, StreamKind};

    #[tokio::test]
    async fn frames_roundtrip_over_a_duplex() {
        let (mut a, mut b) = tokio::io::duplex(64 * 1024);

        let session = Session::open(Host::named("prod-1", true), "bash");
        let hello = ShimToDaemon::Hello {
            version: PROTOCOL_VERSION,
            session: session.clone(),
        };
        let chunk = ShimToDaemon::Chunk {
            chunk: StreamChunk::now(ExecutionId::new(), StreamKind::Stdout, 0, vec![0xff, b'x']),
        };

        write_msg(&mut a, &hello).await.unwrap();
        write_msg(&mut a, &chunk).await.unwrap();
        drop(a); // clean EOF after two frames

        let got1: ShimToDaemon = read_msg(&mut b).await.unwrap().expect("frame 1");
        let got2: ShimToDaemon = read_msg(&mut b).await.unwrap().expect("frame 2");
        assert_eq!(got1, hello);
        assert_eq!(got2, chunk);
        assert!(read_msg::<_, ShimToDaemon>(&mut b).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn daemon_to_shim_roundtrips() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        let msg = DaemonToShim::Input {
            execution: ExecutionId::new(),
            data: b"ls -la\n".to_vec(),
        };
        write_msg(&mut a, &msg).await.unwrap();
        let got: DaemonToShim = read_msg(&mut b).await.unwrap().unwrap();
        assert_eq!(got, msg);
    }
}
