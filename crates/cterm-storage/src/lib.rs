//! # cterm-storage
//!
//! Persistence adapters for [`cterm_core::RecordStore`]:
//!
//! * [`MemoryStore`] — in-process, for the daemon's live cache and tests.
//! * [`SqliteStore`] — durable records on disk.
//!
//! Both encode entities through `serde`, so they stay behaviourally identical;
//! the shared conformance suite in `tests/conformance.rs` runs the same
//! scenario against each.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod memory;
pub mod sqlite;

pub use memory::MemoryStore;
pub use sqlite::SqliteStore;
