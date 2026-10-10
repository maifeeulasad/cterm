//! Live events emitted over an execution's lifetime.
//!
//! The same event stream feeds two consumers: the persistence layer (which must
//! not miss anything) and live subscribers such as agents and the webview
//! (best-effort, may lag). Modelling it as one enum keeps those paths in sync.

use serde::{Deserialize, Serialize};

use crate::record::ExecutionRecord;
use crate::status::ExecutionStatus;
use crate::stream::StreamChunk;

/// Something that happened to an execution, in order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionEvent {
    /// The execution has started running; carries the record in its current state.
    Started(ExecutionRecord),
    /// A chunk of input or output bytes (direction is in the chunk's `kind`).
    Chunk(StreamChunk),
    /// The execution reached a terminal status.
    Status(ExecutionStatus),
}
