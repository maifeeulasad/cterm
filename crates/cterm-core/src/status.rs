//! Lifecycle status of a session and of an individual execution.

use serde::{Deserialize, Serialize};

/// Where an execution is in its lifecycle.
///
/// This is the state agents poll and stream on: "is it still running, did it
/// finish, was it interrupted?". The terminal states carry the outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecutionStatus {
    /// Accepted but not yet started.
    Pending,
    /// Currently executing.
    Running,
    /// Exited on its own. `code` is the process exit code.
    Completed {
        /// The process exit code (0 means success).
        code: i32,
    },
    /// Terminated by a signal (e.g. SIGINT from Ctrl-C, SIGTERM). On platforms
    /// without signals this stays `None`.
    Interrupted {
        /// The POSIX signal number that terminated the process, if known.
        signal: Option<i32>,
    },
    /// The shim could not run or supervise the command at all.
    Failed,
}

impl ExecutionStatus {
    /// Whether this is a terminal (no-further-transition) state.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. } | Self::Interrupted { .. } | Self::Failed
        )
    }

    /// Whether the execution ended successfully (exit code 0).
    #[must_use]
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Completed { code: 0 })
    }
}

/// Where a session is in its lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// The shell is live and accepting commands.
    Active,
    /// The shell exited cleanly.
    Closed,
    /// The shim lost contact with the shell unexpectedly.
    Dropped,
}

impl SessionStatus {
    /// Whether the session can still accept new executions.
    #[must_use]
    pub fn is_open(&self) -> bool {
        matches!(self, Self::Active)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_states_are_terminal() {
        assert!(!ExecutionStatus::Pending.is_terminal());
        assert!(!ExecutionStatus::Running.is_terminal());
        assert!(ExecutionStatus::Completed { code: 0 }.is_terminal());
        assert!(ExecutionStatus::Interrupted { signal: Some(2) }.is_terminal());
        assert!(ExecutionStatus::Failed.is_terminal());
    }

    #[test]
    fn success_requires_zero_exit() {
        assert!(ExecutionStatus::Completed { code: 0 }.is_success());
        assert!(!ExecutionStatus::Completed { code: 1 }.is_success());
        assert!(!ExecutionStatus::Interrupted { signal: Some(2) }.is_success());
    }

    #[test]
    fn status_serializes_tagged() {
        let json = serde_json::to_string(&ExecutionStatus::Completed { code: 2 }).unwrap();
        assert_eq!(json, r#"{"kind":"completed","code":2}"#);
    }
}
