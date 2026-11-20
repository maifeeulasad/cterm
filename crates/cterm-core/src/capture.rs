//! Domain model for **command capture**: the vocabulary for recording *every*
//! command executed on a host, not only the sessions opened under cterm's shim.
//!
//! Background (see issue #19): the shim records a shell it launches, but much of
//! what runs on a host bypasses it — `sh -c`/`bash -c` from editors and build
//! tools, scripts, shells launched by absolute path, static binaries. No single
//! interception point sees everything, so cterm models capture as a set of
//! independent **sources**, each belonging to a **layer**, that emit a common
//! [`CommandEvent`]. The daemon correlates and de-duplicates across them.
//!
//! This module is pure domain: it names the concepts (events, sources, layers,
//! privilege, availability) and holds no I/O. Detecting whether a source can run
//! here, generating its install snippet, and actually capturing events are
//! adapter concerns (see the `cterm-capture` crate), kept behind the ports
//! defined alongside these types.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub mod ports;
pub use ports::{Action, CaptureSource, EnablePlan, HostProbe, PlanStep, RegistryHive};

/// An operating system cterm can capture on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    /// Linux.
    Linux,
    /// macOS (Darwin).
    Macos,
    /// Windows.
    Windows,
}

impl Platform {
    /// The platform this binary is running on, if cterm supports it.
    #[must_use]
    pub fn current() -> Option<Self> {
        match std::env::consts::OS {
            "linux" => Some(Self::Linux),
            "macos" => Some(Self::Macos),
            "windows" => Some(Self::Windows),
            _ => None,
        }
    }
}

/// Which capture strategy a source belongs to. The layers are complementary,
/// not alternatives — see the module docs and issue #19. Ordered from the
/// cheapest / most specific to the most complete / most invasive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    /// cterm's PTY shim — full keystroke+output stream for shells it launches.
    Shim,
    /// A shell's interactive "before each command" hook (bash `DEBUG`, zsh
    /// `preexec`, …). The only layer that sees shell *builtins* and the exact
    /// as-typed command line. Interactive sessions only.
    InteractiveHook,
    /// A shell startup entry honored even for non-interactive `-c` invocations
    /// (bash `BASH_ENV`, cmd `AutoRun`, PowerShell script-block logging). Closes
    /// part of the `sh -c` / `bash -c` bypass.
    NonInteractiveHook,
    /// System-level process/exec auditing (auditd/eBPF, ETW/Event 4688/Sysmon,
    /// Endpoint Security). The only *bypass-proof* layer, but it sees process
    /// launches — blind to builtins that spawn no process.
    SystemAudit,
}

impl Layer {
    /// A short human label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Layer::Shim => "shim",
            Layer::InteractiveHook => "interactive-hook",
            Layer::NonInteractiveHook => "noninteractive-hook",
            Layer::SystemAudit => "system-audit",
        }
    }

    /// Whether this layer can observe shell builtins (`cd`, loops) that execute
    /// no new process. Only the shell-hook layers can.
    #[must_use]
    pub fn sees_builtins(self) -> bool {
        matches!(
            self,
            Layer::Shim | Layer::InteractiveHook | Layer::NonInteractiveHook
        )
    }

    /// Whether this layer is bypass-proof — captures a command however it was
    /// launched. Only system-level exec auditing is.
    #[must_use]
    pub fn is_bypass_proof(self) -> bool {
        matches!(self, Layer::SystemAudit)
    }
}

/// The privilege a source needs to be *enabled*. Elevation is where the config
/// flow must ask the operator to accept or reject.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Privilege {
    /// Installable as the current unprivileged user.
    User,
    /// Needs Windows Administrator.
    Admin,
    /// Needs Unix root (sudo).
    Root,
    /// Needs a specific OS grant, e.g. the macOS Endpoint Security client
    /// entitlement. The string names it.
    Entitlement(String),
}

impl Privilege {
    /// Whether this requires more than the current unprivileged user.
    #[must_use]
    pub fn needs_elevation(&self) -> bool {
        !matches!(self, Privilege::User)
    }

    /// A short human description of what granting this requires.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Privilege::User => "current user".to_owned(),
            Privilege::Admin => "Administrator".to_owned(),
            Privilege::Root => "root (sudo)".to_owned(),
            Privilege::Entitlement(e) => format!("OS grant: {e}"),
        }
    }
}

/// Whether a source can be enabled on this host right now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Availability {
    /// Enableable as the current user, no elevation required.
    Available,
    /// Enableable, but only after the operator grants elevated privilege.
    NeedsElevation {
        /// The privilege that must be granted.
        privilege: Privilege,
    },
    /// A prerequisite is not installed (e.g. Sysmon, the audit package). The
    /// string explains what's missing.
    Missing {
        /// What is missing and, ideally, how to get it.
        reason: String,
    },
    /// Not supported on this platform / OS version at all.
    Unsupported {
        /// Why it can't be used here.
        reason: String,
    },
}

impl Availability {
    /// Whether the source could be enabled at all (possibly after elevation).
    #[must_use]
    pub fn is_enableable(&self) -> bool {
        matches!(
            self,
            Availability::Available | Availability::NeedsElevation { .. }
        )
    }

    /// Whether the source can be enabled with no further privilege.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        matches!(self, Availability::Available)
    }
}

/// A stable identifier for a capture source, e.g. `bash-hook`, `linux-auditd`.
/// Stored in config and used to address a source across runs, so the catalog
/// must keep ids stable.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceId(pub String);

impl SourceId {
    /// Wraps a string id.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The id as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Static description of a capture source: its identity and coverage
/// characteristics, independent of whether it's installed here. The catalog in
/// `cterm-capture` carries one of these per known source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceInfo {
    /// Stable id.
    pub id: SourceId,
    /// Human title, e.g. "bash interactive hook".
    pub title: String,
    /// One-line description of what it captures and how.
    pub summary: String,
    /// Platforms this source applies to.
    pub platforms: Vec<Platform>,
    /// Which capture layer it belongs to.
    pub layer: Layer,
    /// Privilege needed to enable it.
    pub privilege: Privilege,
}

impl SourceInfo {
    /// Whether this source applies to the given platform.
    #[must_use]
    pub fn supports(&self, platform: Platform) -> bool {
        self.platforms.contains(&platform)
    }
}

/// Whether an event marks a command starting or finishing. System-audit sources
/// often only observe the start; shell hooks can report both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// The command is about to run / has just started.
    Started,
    /// The command finished (carries an exit code when known).
    Finished,
}

/// A normalized command-execution event emitted by any [`Layer`]'s source.
///
/// Fields are `Option` where a given source can't supply them: a shell hook
/// knows the as-typed `command`, `cwd`, and `exit_code`; a system exec auditor
/// knows the resolved `argv`, `pid`/`ppid`, and `user` but not builtins. The
/// daemon correlates events across sources by pid + time (see
/// [`CommandEvent::correlation_key`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandEvent {
    /// Which source produced this event.
    pub source: SourceId,
    /// Start or finish.
    pub kind: EventKind,
    /// The command as a single line — as typed for a shell hook, or the argv
    /// joined for an exec auditor.
    pub command: String,
    /// Resolved argument vector, when the source provides one (exec auditors).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub argv: Vec<String>,
    /// Working directory at invocation, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// The shell that hosted the command, if a shell hook (`bash`, `pwsh`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    /// Process id of the command / shell, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Parent process id, if known (for lineage / correlation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ppid: Option<u32>,
    /// The user the command ran as, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Whether the command came from an interactive session.
    pub interactive: bool,
    /// Exit code, on a `Finished` event when the source reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// When the event was observed.
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
}

impl CommandEvent {
    /// A minimal `Started` event with the current timestamp.
    #[must_use]
    pub fn started(source: SourceId, command: impl Into<String>, interactive: bool) -> Self {
        Self {
            source,
            kind: EventKind::Started,
            command: command.into(),
            argv: Vec::new(),
            cwd: None,
            shell: None,
            pid: None,
            ppid: None,
            user: None,
            interactive,
            exit_code: None,
            at: OffsetDateTime::now_utc(),
        }
    }

    /// A coarse key for correlating the same command seen by two sources: the
    /// pid (when known) and the whole-second timestamp. Events without a pid
    /// can't be correlated this way and should be kept as-is.
    #[must_use]
    pub fn correlation_key(&self) -> Option<(u32, i64)> {
        self.pid.map(|pid| (pid, self.at.unix_timestamp()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_coverage_flags() {
        assert!(Layer::InteractiveHook.sees_builtins());
        assert!(!Layer::SystemAudit.sees_builtins());
        assert!(Layer::SystemAudit.is_bypass_proof());
        assert!(!Layer::InteractiveHook.is_bypass_proof());
        // Layers are ordered cheapest -> most complete.
        assert!(Layer::Shim < Layer::SystemAudit);
    }

    #[test]
    fn privilege_elevation() {
        assert!(!Privilege::User.needs_elevation());
        assert!(Privilege::Root.needs_elevation());
        assert!(Privilege::Entitlement("es-client".into()).needs_elevation());
        assert_eq!(Privilege::Root.describe(), "root (sudo)");
    }

    #[test]
    fn availability_predicates() {
        assert!(Availability::Available.is_ready());
        assert!(Availability::Available.is_enableable());
        let needs = Availability::NeedsElevation {
            privilege: Privilege::Admin,
        };
        assert!(!needs.is_ready());
        assert!(needs.is_enableable());
        let missing = Availability::Missing {
            reason: "Sysmon not installed".into(),
        };
        assert!(!missing.is_enableable());
        assert!(!Availability::Unsupported {
            reason: "n/a".into()
        }
        .is_enableable());
    }

    #[test]
    fn source_id_roundtrips_through_json() {
        let id = SourceId::new("bash-hook");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"bash-hook\"");
        assert_eq!(serde_json::from_str::<SourceId>(&json).unwrap(), id);
    }

    #[test]
    fn source_info_supports_platform() {
        let info = SourceInfo {
            id: SourceId::new("bash-hook"),
            title: "bash hook".into(),
            summary: "DEBUG trap".into(),
            platforms: vec![Platform::Linux, Platform::Macos],
            layer: Layer::InteractiveHook,
            privilege: Privilege::User,
        };
        assert!(info.supports(Platform::Linux));
        assert!(!info.supports(Platform::Windows));
    }

    #[test]
    fn command_event_started_and_correlation() {
        let mut ev = CommandEvent::started(SourceId::new("bash-hook"), "ls -la", true);
        assert_eq!(ev.kind, EventKind::Started);
        assert!(ev.correlation_key().is_none());
        ev.pid = Some(4242);
        let (pid, _secs) = ev.correlation_key().unwrap();
        assert_eq!(pid, 4242);
    }

    #[test]
    fn command_event_json_omits_empty_optionals() {
        let ev = CommandEvent::started(SourceId::new("s"), "pwd", false);
        let json = serde_json::to_string(&ev).unwrap();
        // Empty argv and None fields are skipped to keep the wire compact.
        assert!(!json.contains("argv"));
        assert!(!json.contains("cwd"));
        assert!(json.contains("\"command\":\"pwd\""));
    }
}
