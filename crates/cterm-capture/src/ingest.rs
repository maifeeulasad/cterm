//! Turning captured [`CommandEvent`]s into the daemon's record model.
//!
//! Captured commands are recorded as ordinary [`ExecutionRecord`]s against a
//! synthetic "capture" [`Session`], so they show up in the web UI and the gRPC
//! queries alongside shim-recorded sessions — no new storage shape needed. This
//! module is the pure mapping; reading the event log and writing the store is
//! the caller's job (the CLI's `capture ingest`).

use cterm_core::capture::CommandEvent;
use cterm_core::{Command, ExecutionRecord, ExecutionStatus, Host, Session, SessionId};

/// The shell label used for the synthetic capture session, so it's
/// distinguishable from real shells in the UI.
pub const CAPTURE_SHELL: &str = "capture";

/// Opens a synthetic local session that captured commands are recorded against.
#[must_use]
pub fn capture_session() -> Session {
    Session::open(Host::local(), CAPTURE_SHELL)
}

/// Converts a captured command event into an execution record in `session`.
///
/// The as-typed command, shell, and cwd come from the event, and the record's
/// timestamps are stamped from the event (not "now") so the audit trail is
/// faithful. A `Finished` event with an exit code becomes a `Completed` status;
/// a `Started` event is left `Running` (we may never see it finish).
#[must_use]
pub fn event_to_execution(event: &CommandEvent, session: SessionId) -> ExecutionRecord {
    let shell = event
        .shell
        .clone()
        .unwrap_or_else(|| CAPTURE_SHELL.to_owned());
    let mut command = Command::new(event.command.clone(), shell);
    if let Some(cwd) = &event.cwd {
        command = command.with_cwd(cwd.clone());
    }

    let mut rec = ExecutionRecord::open(session, command);
    rec.started_at = event.at;
    match event.exit_code {
        Some(code) => {
            rec.transition(ExecutionStatus::Completed { code });
            rec.ended_at = Some(event.at);
        }
        // Seen starting but not finishing: Running is the honest status (the
        // interactive hook only reports preexec today).
        None => rec.transition(ExecutionStatus::Running),
    }
    rec
}

#[cfg(test)]
mod tests {
    use super::*;
    use cterm_core::capture::{EventKind, SourceId};

    fn finished_event() -> CommandEvent {
        let mut ev = CommandEvent::started(SourceId::new("bash-hook"), "git status", true);
        ev.shell = Some("bash".to_owned());
        ev.cwd = Some("/home/me".to_owned());
        ev.kind = EventKind::Finished;
        ev.exit_code = Some(0);
        ev
    }

    #[test]
    fn maps_command_shell_and_cwd() {
        let session = capture_session();
        let rec = event_to_execution(&finished_event(), session.id);
        assert_eq!(rec.command.raw, "git status");
        assert_eq!(rec.command.shell, "bash");
        assert_eq!(rec.command.cwd.as_deref(), Some("/home/me"));
        assert_eq!(rec.session, session.id);
    }

    #[test]
    fn finished_event_is_completed_with_event_time() {
        let ev = finished_event();
        let rec = event_to_execution(&ev, capture_session().id);
        assert!(matches!(rec.status, ExecutionStatus::Completed { code: 0 }));
        assert_eq!(rec.started_at, ev.at);
        assert_eq!(rec.ended_at, Some(ev.at));
    }

    #[test]
    fn started_event_stays_running() {
        let ev = CommandEvent::started(SourceId::new("s"), "sleep 10", true);
        let rec = event_to_execution(&ev, capture_session().id);
        assert!(matches!(rec.status, ExecutionStatus::Running));
        assert!(rec.ended_at.is_none());
        // Missing shell falls back to the capture label.
        assert_eq!(rec.command.shell, CAPTURE_SHELL);
    }

    #[test]
    fn capture_session_is_labeled() {
        assert_eq!(capture_session().shell, CAPTURE_SHELL);
    }
}
