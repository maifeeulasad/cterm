//! The durable record of one execution.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::command::Command;
use crate::ids::{ExecutionId, SessionId};
use crate::status::ExecutionStatus;

/// The authoritative record of a single command execution.
///
/// The shim reports lifecycle transitions and the daemon maintains this record,
/// which is what gets persisted and what the webview and audit trail read. Byte
/// streams are stored separately (see [`crate::stream::StreamChunk`]) and
/// referenced by [`ExecutionRecord::id`] — a long-running `tail -f` must not
/// bloat a single row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionRecord {
    /// Unique id of this execution.
    pub id: ExecutionId,
    /// Session this execution belongs to.
    pub session: SessionId,
    /// What was run.
    pub command: Command,
    /// Current lifecycle status.
    pub status: ExecutionStatus,
    /// When the execution was first accepted.
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    /// When the execution reached a terminal status, if it has.
    #[serde(with = "time::serde::rfc3339::option", default)]
    pub ended_at: Option<OffsetDateTime>,
}

impl ExecutionRecord {
    /// Opens a new record in [`ExecutionStatus::Pending`], stamped now.
    #[must_use]
    pub fn open(session: SessionId, command: Command) -> Self {
        Self {
            id: ExecutionId::new(),
            session,
            command,
            status: ExecutionStatus::Pending,
            started_at: OffsetDateTime::now_utc(),
            ended_at: None,
        }
    }

    /// Applies a status transition, stamping `ended_at` when it becomes terminal.
    ///
    /// Terminal states are sticky: once set, a later transition does not clear
    /// `ended_at`, and the first terminal timestamp wins.
    pub fn transition(&mut self, status: ExecutionStatus) {
        if status.is_terminal() && self.ended_at.is_none() {
            self.ended_at = Some(OffsetDateTime::now_utc());
        }
        self.status = status;
    }

    /// Wall-clock duration if the execution has ended.
    #[must_use]
    pub fn duration(&self) -> Option<time::Duration> {
        self.ended_at.map(|end| end - self.started_at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ExecutionRecord {
        ExecutionRecord::open(SessionId::new(), Command::new("echo hi", "bash"))
    }

    #[test]
    fn opens_pending_without_end() {
        let rec = sample();
        assert_eq!(rec.status, ExecutionStatus::Pending);
        assert!(rec.ended_at.is_none());
        assert!(rec.duration().is_none());
    }

    #[test]
    fn terminal_transition_stamps_end_once() {
        let mut rec = sample();
        rec.transition(ExecutionStatus::Running);
        assert!(rec.ended_at.is_none());

        rec.transition(ExecutionStatus::Completed { code: 0 });
        let first_end = rec.ended_at.expect("end stamped on terminal");
        assert!(rec.duration().is_some());

        // A subsequent terminal transition must not move the recorded end time.
        rec.transition(ExecutionStatus::Failed);
        assert_eq!(rec.ended_at, Some(first_end));
    }

    #[test]
    fn roundtrips_through_json() {
        let rec = sample();
        let json = serde_json::to_string(&rec).unwrap();
        let back: ExecutionRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(rec, back);
    }
}
