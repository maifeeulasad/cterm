//! A terminal session: one shell lifetime on one host.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::ids::SessionId;
use crate::status::SessionStatus;

/// Describes where a session is running. For the hybrid-SSH use case this is
/// how an agent tells a prod session apart from a local one without ever
/// holding the credentials used to open it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    /// Human label, e.g. `prod-web-1` or `localhost`.
    pub name: String,
    /// Whether this host is considered production. Agents may be policy-limited
    /// to observe-only on production sessions.
    #[serde(default)]
    pub is_production: bool,
}

impl Host {
    /// A local, non-production host.
    #[must_use]
    pub fn local() -> Self {
        Self {
            name: "localhost".to_owned(),
            is_production: false,
        }
    }

    /// A named host, marking whether it is production.
    #[must_use]
    pub fn named(name: impl Into<String>, is_production: bool) -> Self {
        Self {
            name: name.into(),
            is_production,
        }
    }
}

/// A terminal session the daemon is tracking.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// Unique id of this session.
    pub id: SessionId,
    /// Where it runs.
    pub host: Host,
    /// The shell program backing it, e.g. `zsh`.
    pub shell: String,
    /// Current lifecycle status.
    pub status: SessionStatus,
    /// When the session opened.
    #[serde(with = "time::serde::rfc3339")]
    pub opened_at: OffsetDateTime,
    /// When it closed, if it has.
    #[serde(with = "time::serde::rfc3339::option", default)]
    pub closed_at: Option<OffsetDateTime>,
}

impl Session {
    /// Opens a new active session stamped now.
    #[must_use]
    pub fn open(host: Host, shell: impl Into<String>) -> Self {
        Self {
            id: SessionId::new(),
            host,
            shell: shell.into(),
            status: SessionStatus::Active,
            opened_at: OffsetDateTime::now_utc(),
            closed_at: None,
        }
    }

    /// Marks the session closed/dropped, stamping `closed_at` once.
    pub fn close(&mut self, status: SessionStatus) {
        debug_assert!(!status.is_open(), "close() takes a terminal status");
        if self.closed_at.is_none() {
            self.closed_at = Some(OffsetDateTime::now_utc());
        }
        self.status = status;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_active() {
        let s = Session::open(Host::local(), "zsh");
        assert!(s.status.is_open());
        assert!(s.closed_at.is_none());
        assert!(!s.host.is_production);
    }

    #[test]
    fn close_stamps_once() {
        let mut s = Session::open(Host::named("prod-1", true), "bash");
        s.close(SessionStatus::Closed);
        let first = s.closed_at;
        assert!(first.is_some());
        s.close(SessionStatus::Dropped);
        assert_eq!(s.closed_at, first);
        assert_eq!(s.status, SessionStatus::Dropped);
    }
}
