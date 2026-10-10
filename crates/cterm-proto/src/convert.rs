//! Conversions from `cterm-core` domain types to their protobuf wire form.
//!
//! Only core→proto is provided: the daemon produces these messages, and
//! requests coming the other way carry plain scalars the daemon parses itself
//! (e.g. ids via `FromStr`). Timestamps are rendered as RFC3339 strings.

use cterm_core as core;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::v1;

fn ts(t: OffsetDateTime) -> String {
    t.format(&Rfc3339).unwrap_or_default()
}

fn ts_opt(t: Option<OffsetDateTime>) -> String {
    t.map(ts).unwrap_or_default()
}

/// Maps a stream direction.
#[must_use]
pub fn stream_kind(k: core::StreamKind) -> v1::StreamKind {
    match k {
        core::StreamKind::Stdin => v1::StreamKind::Stdin,
        core::StreamKind::Stdout => v1::StreamKind::Stdout,
        core::StreamKind::Stderr => v1::StreamKind::Stderr,
    }
}

/// Maps an execution status.
#[must_use]
pub fn status(s: core::ExecutionStatus) -> v1::Status {
    use core::ExecutionStatus as E;
    use v1::status::Kind;

    let mut out = v1::Status::default();
    match s {
        E::Pending => out.kind = Kind::Pending as i32,
        E::Running => out.kind = Kind::Running as i32,
        E::Completed { code } => {
            out.kind = Kind::Completed as i32;
            out.code = code;
        }
        E::Interrupted { signal } => {
            out.kind = Kind::Interrupted as i32;
            out.has_signal = signal.is_some();
            out.signal = signal.unwrap_or(0);
        }
        E::Failed => out.kind = Kind::Failed as i32,
    }
    out
}

/// Maps a host descriptor.
#[must_use]
pub fn host(h: &core::Host) -> v1::Host {
    v1::Host {
        name: h.name.clone(),
        is_production: h.is_production,
    }
}

/// The lowercase string form of a session status used on the wire.
#[must_use]
pub fn session_status(s: core::SessionStatus) -> String {
    match s {
        core::SessionStatus::Active => "active",
        core::SessionStatus::Closed => "closed",
        core::SessionStatus::Dropped => "dropped",
    }
    .to_owned()
}

/// Maps a session.
#[must_use]
pub fn session(s: &core::Session) -> v1::Session {
    v1::Session {
        id: s.id.to_string(),
        host: Some(host(&s.host)),
        shell: s.shell.clone(),
        status: session_status(s.status),
        opened_at: ts(s.opened_at),
        closed_at: ts_opt(s.closed_at),
    }
}

/// Maps a command.
#[must_use]
pub fn command(c: &core::Command) -> v1::Command {
    v1::Command {
        raw: c.raw.clone(),
        cwd: c.cwd.clone().unwrap_or_default(),
        shell: c.shell.clone(),
    }
}

/// Maps an execution record.
#[must_use]
pub fn record(r: &core::ExecutionRecord) -> v1::ExecutionRecord {
    v1::ExecutionRecord {
        id: r.id.to_string(),
        session: r.session.to_string(),
        command: Some(command(&r.command)),
        status: Some(status(r.status)),
        started_at: ts(r.started_at),
        ended_at: ts_opt(r.ended_at),
    }
}

/// Maps a stream chunk.
#[must_use]
pub fn chunk(c: &core::StreamChunk) -> v1::StreamChunk {
    v1::StreamChunk {
        execution: c.execution.to_string(),
        kind: stream_kind(c.kind) as i32,
        seq: c.seq,
        data: c.data.clone(),
        at: ts(c.at),
    }
}

/// Maps a live execution event.
#[must_use]
pub fn event(e: &core::ExecutionEvent) -> v1::ExecutionEvent {
    use core::ExecutionEvent as Ev;
    use v1::execution_event::Event;

    let event = match e {
        Ev::Started(r) => Event::Started(record(r)),
        Ev::Chunk(c) => Event::Chunk(chunk(c)),
        Ev::Status(s) => Event::Status(status(*s)),
    };
    v1::ExecutionEvent { event: Some(event) }
}
