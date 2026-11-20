//! Ports for command capture — the abstract seams the `cterm-capture` adapters
//! implement. Keeping these in the domain crate lets the CLI and daemon depend
//! on the *shape* of a capture source without pulling in any OS-specific code,
//! and lets tests substitute fakes for the host.

use super::{Availability, Platform, SourceInfo};

/// A read-only view of the host environment.
///
/// Source availability detection ("is auditd installed?", "is `BASH_ENV`
/// already set?", "am I root?") needs to inspect the machine. Routing every
/// such query through this port keeps the sources pure and unit-testable: real
/// code uses a live probe, tests pass a fake with canned answers.
pub trait HostProbe {
    /// The host OS, or `None` if cterm doesn't support it.
    fn platform(&self) -> Option<Platform>;
    /// Whether the current process already holds admin / root.
    fn is_elevated(&self) -> bool;
    /// The value of an environment variable, if set.
    fn env(&self, key: &str) -> Option<String>;
    /// Whether a path exists on disk.
    fn path_exists(&self, path: &str) -> bool;
    /// Whether an executable named `program` is resolvable on `PATH`.
    fn has_program(&self, program: &str) -> bool;
}

/// A Windows registry hive an action targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryHive {
    /// `HKEY_CURRENT_USER` — writable by the current user.
    CurrentUser,
    /// `HKEY_LOCAL_MACHINE` — needs Administrator.
    LocalMachine,
}

/// One declarative step in enabling (or disabling) a source.
///
/// Actions are *data, not code*: the config flow can render exactly what will
/// change before the operator accepts, and a separate executor (an adapter
/// concern) performs them. This is what makes "may ask the user for elevated
/// permission, which they can accept or reject" a first-class, inspectable step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Create or overwrite a file with the given contents.
    WriteFile {
        /// Target path.
        path: String,
        /// Full file contents.
        contents: String,
    },
    /// Idempotently insert a block into a file, guarded by a unique marker so
    /// re-running doesn't duplicate it and disabling can remove just that block.
    AppendOnce {
        /// Target path.
        path: String,
        /// Unique marker identifying the cterm-managed block.
        marker: String,
        /// The block to insert between marker lines.
        block: String,
    },
    /// Set a Windows registry value.
    SetRegistryValue {
        /// Hive.
        hive: RegistryHive,
        /// Subkey path.
        key: String,
        /// Value name.
        name: String,
        /// Value data.
        value: String,
    },
    /// Run a command (e.g. `auditctl`, installing an audit rule).
    RunCommand {
        /// Program.
        program: String,
        /// Arguments.
        args: Vec<String>,
    },
    /// Delete a file (undoing a [`Action::WriteFile`]).
    RemoveFile {
        /// Path to delete; a missing file is not an error.
        path: String,
    },
    /// Remove a marker-guarded block from a file (undoing [`Action::AppendOnce`]),
    /// leaving the rest of the file intact.
    RemoveBlock {
        /// Target path.
        path: String,
        /// The marker whose block to remove.
        marker: String,
    },
    /// Delete a Windows registry value (undoing [`Action::SetRegistryValue`]).
    DeleteRegistryValue {
        /// Hive.
        hive: RegistryHive,
        /// Subkey path.
        key: String,
        /// Value name.
        name: String,
    },
    /// Something the operator must do by hand (e.g. grant a macOS entitlement);
    /// the string is the instructions.
    Manual {
        /// What the operator should do.
        instructions: String,
    },
}

/// A single step in an enable/disable plan: what it does, in human words, and
/// whether performing it needs elevated privilege.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanStep {
    /// Human-readable description, shown in the accept/reject prompt.
    pub description: String,
    /// The declarative action.
    pub action: Action,
    /// Whether this step needs admin / root to perform.
    pub elevated: bool,
}

impl PlanStep {
    /// A convenience constructor.
    #[must_use]
    pub fn new(description: impl Into<String>, action: Action, elevated: bool) -> Self {
        Self {
            description: description.into(),
            action,
            elevated,
        }
    }
}

/// An ordered set of steps to enable or disable a source on this host.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnablePlan {
    /// The steps, applied in order.
    pub steps: Vec<PlanStep>,
}

impl EnablePlan {
    /// A plan from a list of steps.
    #[must_use]
    pub fn new(steps: Vec<PlanStep>) -> Self {
        Self { steps }
    }

    /// Whether any step needs elevation — i.e. the operator must be prompted to
    /// grant admin/root before this plan can be applied.
    #[must_use]
    pub fn requires_elevation(&self) -> bool {
        self.steps.iter().any(|s| s.elevated)
    }

    /// Whether the plan does nothing (e.g. already enabled, or nothing to undo).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

/// A concrete capture mechanism — a shell hook, a system exec auditor, …
///
/// A source knows its own identity and coverage ([`SourceInfo`]), can tell
/// whether it's usable on a given host ([`CaptureSource::availability`]), and
/// can produce the declarative [`EnablePlan`] to turn itself on or off. It does
/// *not* perform I/O or capture events directly — those are separate adapter
/// roles, so a source stays a small, testable unit of policy.
pub trait CaptureSource {
    /// Static identity and coverage characteristics.
    fn info(&self) -> SourceInfo;

    /// Whether this source can be enabled on the probed host (and at what
    /// privilege).
    fn availability(&self, probe: &dyn HostProbe) -> Availability;

    /// The steps to enable this source on the probed host.
    fn enable_plan(&self, probe: &dyn HostProbe) -> EnablePlan;

    /// The steps to disable / undo it on the probed host.
    fn disable_plan(&self, probe: &dyn HostProbe) -> EnablePlan;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{Layer, Privilege, SourceId};
    use std::collections::{HashMap, HashSet};

    /// A canned host for tests.
    #[derive(Default)]
    struct FakeProbe {
        platform: Option<Platform>,
        elevated: bool,
        env: HashMap<String, String>,
        paths: HashSet<String>,
        programs: HashSet<String>,
    }

    impl HostProbe for FakeProbe {
        fn platform(&self) -> Option<Platform> {
            self.platform
        }
        fn is_elevated(&self) -> bool {
            self.elevated
        }
        fn env(&self, key: &str) -> Option<String> {
            self.env.get(key).cloned()
        }
        fn path_exists(&self, path: &str) -> bool {
            self.paths.contains(path)
        }
        fn has_program(&self, program: &str) -> bool {
            self.programs.contains(program)
        }
    }

    /// A toy source: available on Linux as the current user, unsupported else.
    struct ToySource;

    impl CaptureSource for ToySource {
        fn info(&self) -> SourceInfo {
            SourceInfo {
                id: SourceId::new("toy"),
                title: "toy".into(),
                summary: "test source".into(),
                platforms: vec![Platform::Linux],
                layer: Layer::InteractiveHook,
                privilege: Privilege::User,
            }
        }
        fn availability(&self, probe: &dyn HostProbe) -> Availability {
            match probe.platform() {
                Some(Platform::Linux) => Availability::Available,
                _ => Availability::Unsupported {
                    reason: "Linux only".into(),
                },
            }
        }
        fn enable_plan(&self, _probe: &dyn HostProbe) -> EnablePlan {
            EnablePlan::new(vec![PlanStep::new(
                "write hook file",
                Action::WriteFile {
                    path: "/tmp/hook".into(),
                    contents: "x".into(),
                },
                false,
            )])
        }
        fn disable_plan(&self, _probe: &dyn HostProbe) -> EnablePlan {
            EnablePlan::default()
        }
    }

    #[test]
    fn source_uses_probe_for_availability() {
        let linux = FakeProbe {
            platform: Some(Platform::Linux),
            ..Default::default()
        };
        let windows = FakeProbe {
            platform: Some(Platform::Windows),
            ..Default::default()
        };
        assert_eq!(ToySource.availability(&linux), Availability::Available);
        assert!(!ToySource.availability(&windows).is_enableable());
    }

    #[test]
    fn plan_elevation_and_emptiness() {
        let p = FakeProbe::default();
        let enable = ToySource.enable_plan(&p);
        assert!(!enable.is_empty());
        assert!(!enable.requires_elevation());
        assert!(ToySource.disable_plan(&p).is_empty());

        let elevated = EnablePlan::new(vec![PlanStep::new(
            "install audit rule",
            Action::RunCommand {
                program: "auditctl".into(),
                args: vec!["-a".into()],
            },
            true,
        )]);
        assert!(elevated.requires_elevation());
    }

    #[test]
    fn probe_fake_answers() {
        let p = FakeProbe {
            elevated: true,
            env: HashMap::from([("BASH_ENV".into(), "/x".into())]),
            programs: HashSet::from(["auditctl".into()]),
            ..Default::default()
        };
        assert!(p.is_elevated());
        assert_eq!(p.env("BASH_ENV").as_deref(), Some("/x"));
        assert!(p.has_program("auditctl"));
        assert!(!p.has_program("sysmon"));
        assert!(!p.path_exists("/nope"));
    }
}
