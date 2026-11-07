//! A terminal session: one shell lifetime on one host.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::error::{Error, Result};
use crate::ids::SessionId;
use crate::status::SessionStatus;

/// Hard cap on a session title's length, in bytes. Keeps listings and the
/// webview readable; the daemon rejects anything larger with a clear error
/// rather than silently truncating.
pub const MAX_TITLE_BYTES: usize = 256;

/// Hard cap on the number of labels on a single session. Matches the "few
/// pieces of metadata" model — not an attribute store.
pub const MAX_LABELS: usize = 32;

/// Hard cap on a single label key or value, in bytes.
pub const MAX_LABEL_BYTES: usize = 128;

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
///
/// `title` and `labels` are optional caller-provided metadata that rides
/// alongside the session in listings and the webview — they do not affect
/// scheduling, storage identity, or how the shell runs. Both fields default
/// to empty so older stored sessions (which lack them on disk) continue to
/// deserialize unchanged.
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
    /// Human-readable one-line title for the session (empty = unset). Shown
    /// alongside the id in listings and the webview.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Caller-supplied key/value metadata. Small, stringly-typed, intended
    /// for things like `env=prod` or `ticket=ABC-123`. Not indexed.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
}

impl Session {
    /// Opens a new active session stamped now, with no title and no labels.
    #[must_use]
    pub fn open(host: Host, shell: impl Into<String>) -> Self {
        Self {
            id: SessionId::new(),
            host,
            shell: shell.into(),
            status: SessionStatus::Active,
            opened_at: OffsetDateTime::now_utc(),
            closed_at: None,
            title: String::new(),
            labels: BTreeMap::new(),
        }
    }

    /// Attaches a title to this session (builder-style). Call sites validate
    /// user-supplied titles with [`validate_title`] first; this setter
    /// trusts its input to keep the struct cheap to build in tests.
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Replaces this session's labels (builder-style). Call sites validate
    /// user-supplied labels with [`validate_labels`] first.
    #[must_use]
    pub fn with_labels(mut self, labels: BTreeMap<String, String>) -> Self {
        self.labels = labels;
        self
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

/// Validate a caller-supplied session title. Returns the trimmed, canonical
/// form on success. Empty input is rejected: the opener should just omit
/// `title` to leave a session untitled.
///
/// Rules: non-empty, at most [`MAX_TITLE_BYTES`] bytes, no control
/// characters (newlines, NULs, escape sequences) — a title lands in UI
/// surfaces and we don't want a `\x1b]0;...` payload masquerading as one.
pub fn validate_title(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Error::Other("title must not be empty".into()));
    }
    if trimmed.len() > MAX_TITLE_BYTES {
        return Err(Error::Other(format!(
            "title too long: {} bytes (max {MAX_TITLE_BYTES})",
            trimmed.len()
        )));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(Error::Other(
            "title must not contain control characters".into(),
        ));
    }
    Ok(trimmed.to_owned())
}

/// Validate a caller-supplied label map. Returns the canonical form (same
/// keys/values; input pre-validated per entry) on success.
///
/// Rules per entry: key non-empty, key/value within [`MAX_LABEL_BYTES`],
/// no control characters in either, no `=` in keys (so CLI `--label k=v`
/// stays unambiguous). At most [`MAX_LABELS`] entries total.
pub fn validate_labels(raw: BTreeMap<String, String>) -> Result<BTreeMap<String, String>> {
    if raw.len() > MAX_LABELS {
        return Err(Error::Other(format!(
            "too many labels: {} (max {MAX_LABELS})",
            raw.len()
        )));
    }
    for (k, v) in &raw {
        if k.is_empty() {
            return Err(Error::Other("label key must not be empty".into()));
        }
        if k.len() > MAX_LABEL_BYTES {
            return Err(Error::Other(format!(
                "label key too long: {} bytes (max {MAX_LABEL_BYTES})",
                k.len()
            )));
        }
        if v.len() > MAX_LABEL_BYTES {
            return Err(Error::Other(format!(
                "label value too long: {} bytes (max {MAX_LABEL_BYTES})",
                v.len()
            )));
        }
        if k.chars().any(char::is_control) || v.chars().any(char::is_control) {
            return Err(Error::Other(
                "labels must not contain control characters".into(),
            ));
        }
        if k.contains('=') {
            return Err(Error::Other(
                "label key must not contain '=' (reserved for CLI parsing)".into(),
            ));
        }
    }
    Ok(raw)
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
        assert!(s.title.is_empty());
        assert!(s.labels.is_empty());
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

    #[test]
    fn title_and_labels_default_absent_in_json() {
        let s = Session::open(Host::local(), "zsh");
        let j = serde_json::to_string(&s).unwrap();
        // skip_serializing_if = empty keeps older consumers happy
        assert!(!j.contains("\"title\""));
        assert!(!j.contains("\"labels\""));
    }

    #[test]
    fn title_and_labels_roundtrip_when_set() {
        let mut labels = BTreeMap::new();
        labels.insert("env".to_owned(), "prod".to_owned());
        let s = Session::open(Host::local(), "zsh")
            .with_title("deploy webapp")
            .with_labels(labels);
        let j = serde_json::to_string(&s).unwrap();
        let back: Session = serde_json::from_str(&j).unwrap();
        assert_eq!(back.title, "deploy webapp");
        assert_eq!(back.labels.get("env"), Some(&"prod".to_owned()));
    }

    #[test]
    fn deserializes_from_pre_metadata_blob() {
        // A row written by cterm <= 0.5.x — no title/labels fields.
        // SessionId serializes transparently as a bare uuid.
        let legacy = r#"{
            "id":"00000000-0000-0000-0000-000000000000",
            "host":{"name":"localhost","is_production":false},
            "shell":"zsh",
            "status":"active",
            "opened_at":"2026-11-07T00:00:00Z"
        }"#;
        let s: Session = serde_json::from_str(legacy).unwrap();
        assert!(s.title.is_empty());
        assert!(s.labels.is_empty());
    }

    #[test]
    fn validate_title_accepts_trims_and_rejects() {
        assert_eq!(validate_title("  hello  ").unwrap(), "hello");
        assert!(validate_title("").is_err());
        assert!(validate_title("   ").is_err());
        assert!(validate_title("bad\nline").is_err());
        assert!(validate_title("bad\x1b[31mred").is_err());
        let long = "a".repeat(MAX_TITLE_BYTES + 1);
        assert!(validate_title(&long).is_err());
    }

    #[test]
    fn validate_labels_rules() {
        // happy path
        let mut m = BTreeMap::new();
        m.insert("env".into(), "prod".into());
        m.insert("ticket".into(), "ABC-123".into());
        assert!(validate_labels(m.clone()).is_ok());

        // empty key rejected
        let mut bad_key = BTreeMap::new();
        bad_key.insert(String::new(), "v".into());
        assert!(validate_labels(bad_key).is_err());

        // '=' in key rejected
        let mut eq = BTreeMap::new();
        eq.insert("k=1".into(), "v".into());
        assert!(validate_labels(eq).is_err());

        // control chars rejected
        let mut ctl = BTreeMap::new();
        ctl.insert("k".into(), "line\nbreak".into());
        assert!(validate_labels(ctl).is_err());

        // oversized key
        let big = "x".repeat(MAX_LABEL_BYTES + 1);
        let mut huge = BTreeMap::new();
        huge.insert(big, "v".into());
        assert!(validate_labels(huge).is_err());

        // too many
        let many: BTreeMap<_, _> =
            (0..=MAX_LABELS as u32).map(|i| (format!("k{i}"), "v".into())).collect();
        assert!(validate_labels(many).is_err());
    }
}
