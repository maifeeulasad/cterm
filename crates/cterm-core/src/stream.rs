//! Byte streams flowing to/from a running execution.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::ids::ExecutionId;

/// Which direction a chunk of bytes was flowing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamKind {
    /// Bytes written *to* the process (what was typed / sent by an agent).
    Stdin,
    /// Bytes the process wrote to standard output.
    Stdout,
    /// Bytes the process wrote to standard error.
    Stderr,
}

/// One ordered chunk of bytes belonging to an execution's stream.
///
/// Chunks are the unit of realtime streaming. `seq` is monotonic *per
/// execution* so a consumer that reconnects can resume and detect gaps; the
/// daemon, not the shim, is the authority on ordering across stdout/stderr.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamChunk {
    /// Execution this chunk belongs to.
    pub execution: ExecutionId,
    /// Which stream the bytes came from / went to.
    pub kind: StreamKind,
    /// Monotonic sequence number within the execution, starting at 0.
    pub seq: u64,
    /// Raw bytes. We keep bytes, not `String`: terminal output is not always
    /// valid UTF-8 (control sequences, binary `cat`, partial multibyte reads).
    #[serde(with = "crate::serde_bytes_base64")]
    pub data: Vec<u8>,
    /// When the daemon received the chunk.
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
}

impl StreamChunk {
    /// Convenience constructor stamping `at` with the current time.
    #[must_use]
    pub fn now(execution: ExecutionId, kind: StreamKind, seq: u64, data: Vec<u8>) -> Self {
        Self {
            execution,
            kind,
            seq,
            data,
            at: OffsetDateTime::now_utc(),
        }
    }

    /// A best-effort lossy string view, for logs and the webview.
    #[must_use]
    pub fn text_lossy(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_json_with_non_utf8_bytes() {
        let chunk = StreamChunk::now(ExecutionId::new(), StreamKind::Stdout, 0, vec![0xff, 0x00, 0x41]);
        let json = serde_json::to_string(&chunk).unwrap();
        let back: StreamChunk = serde_json::from_str(&json).unwrap();
        assert_eq!(chunk, back);
        assert_eq!(back.data, vec![0xff, 0x00, 0x41]);
    }

    #[test]
    fn text_lossy_replaces_invalid() {
        let chunk = StreamChunk::now(ExecutionId::new(), StreamKind::Stdout, 0, vec![0xff, b'A']);
        assert!(chunk.text_lossy().contains('A'));
    }
}
