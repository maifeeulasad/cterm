//! The catalog's concrete [`CaptureSource`] implementations.
//!
//! Each source is a small, declarative unit: it knows which platform/shell it
//! targets, how to tell whether it's usable here (via the [`HostProbe`]), and
//! the [`EnablePlan`]/`disable_plan` to turn itself on or off. The actual I/O
//! (applying a plan) and event transport live elsewhere — a source performs no
//! side effects, which keeps it fully unit-testable against a fake probe.

use cterm_core::capture::{
    Action, Availability, CaptureSource, EnablePlan, HostProbe, Layer, PlanStep, Privilege,
    RegistryHive, SourceId, SourceInfo,
};
use cterm_core::Platform;

/// The per-user directory cterm installs capture hooks into (`~/.cterm/hooks`).
const HOOK_DIR: &str = ".cterm/hooks";

/// The home directory from the probe (`HOME`, else `USERPROFILE`).
fn home(probe: &dyn HostProbe) -> Option<String> {
    probe.env("HOME").or_else(|| probe.env("USERPROFILE"))
}

/// Marker lines bracketing a cterm-managed block in a user rc file.
fn marker(shell: &str) -> String {
    format!("cterm-capture-{shell}")
}

/// A shell interactive/non-interactive hook source. Shares the same shape for
/// bash and zsh; the differences (rc file, hook body, which layer it reaches,
/// whether it also covers `-c`) are carried in the fields.
struct ShellHookSource {
    id: &'static str,
    title: &'static str,
    summary: &'static str,
    shell: &'static str,
    /// rc file under $HOME to source the hook from (interactive).
    rc_file: &'static str,
    /// Hook script file name under the hook dir.
    hook_file: &'static str,
    /// Hook script contents.
    hook_body: &'static str,
    /// rc block that loads the hook (and, for bash, exports BASH_ENV).
    rc_block: &'static str,
    layer: Layer,
}

impl ShellHookSource {
    fn hook_path(&self, home: &str) -> String {
        format!("{home}/{HOOK_DIR}/{}", self.hook_file)
    }
    fn rc_path(&self, home: &str) -> String {
        format!("{home}/{}", self.rc_file)
    }
}

impl CaptureSource for ShellHookSource {
    fn info(&self) -> SourceInfo {
        SourceInfo {
            id: SourceId::new(self.id),
            title: self.title.to_owned(),
            summary: self.summary.to_owned(),
            platforms: vec![Platform::Linux, Platform::Macos],
            layer: self.layer,
            privilege: Privilege::User,
        }
    }

    fn availability(&self, probe: &dyn HostProbe) -> Availability {
        match probe.platform() {
            Some(Platform::Linux | Platform::Macos) => {}
            _ => {
                return Availability::Unsupported {
                    reason: format!("{} hooks apply to Linux and macOS", self.shell),
                }
            }
        }
        if home(probe).is_none() {
            return Availability::Unsupported {
                reason: "neither HOME nor USERPROFILE is set".to_owned(),
            };
        }
        if !probe.has_program(self.shell) {
            return Availability::Missing {
                reason: format!("{} is not installed / not on PATH", self.shell),
            };
        }
        Availability::Available
    }

    fn enable_plan(&self, probe: &dyn HostProbe) -> EnablePlan {
        let Some(home) = home(probe) else {
            return EnablePlan::default();
        };
        EnablePlan::new(vec![
            PlanStep::new(
                format!("install the {} capture hook", self.shell),
                Action::WriteFile {
                    path: self.hook_path(&home),
                    contents: self.hook_body.to_owned(),
                },
                false,
            ),
            PlanStep::new(
                format!("load the hook from {}", self.rc_file),
                Action::AppendOnce {
                    path: self.rc_path(&home),
                    marker: marker(self.shell),
                    block: self.rc_block.to_owned(),
                },
                false,
            ),
        ])
    }

    fn disable_plan(&self, probe: &dyn HostProbe) -> EnablePlan {
        let Some(home) = home(probe) else {
            return EnablePlan::default();
        };
        EnablePlan::new(vec![
            PlanStep::new(
                format!("stop loading the hook from {}", self.rc_file),
                Action::RemoveBlock {
                    path: self.rc_path(&home),
                    marker: marker(self.shell),
                },
                false,
            ),
            PlanStep::new(
                format!("remove the {} capture hook", self.shell),
                Action::RemoveFile {
                    path: self.hook_path(&home),
                },
                false,
            ),
        ])
    }
}

const BASH_HOOK: &str = r#"# cterm bash command-capture hook (managed by `cterm capture`).
# Reports each command to the cterm daemon. Loaded for interactive shells and,
# via BASH_ENV, for non-interactive `bash -c` invocations too.
if [ -z "${__CTERM_CAPTURE_LOADED:-}" ]; then
  __CTERM_CAPTURE_LOADED=1
  __cterm_capture_preexec() {
    [ -n "${COMP_LINE:-}" ] && return
    case "$BASH_COMMAND" in __cterm_capture_preexec) return ;; esac
    cterm capture emit --shell bash --command "$BASH_COMMAND" --cwd "$PWD" >/dev/null 2>&1
  }
  trap '__cterm_capture_preexec' DEBUG
fi
"#;

const BASH_RC_BLOCK: &str = r#"[ -f "$HOME/.cterm/hooks/bash-capture.sh" ] && . "$HOME/.cterm/hooks/bash-capture.sh"
export BASH_ENV="$HOME/.cterm/hooks/bash-capture.sh""#;

const ZSH_HOOK: &str = r#"# cterm zsh command-capture hook (managed by `cterm capture`).
# Reports each interactive command to the cterm daemon via `cterm capture emit`.
if [[ -z ${__CTERM_CAPTURE_LOADED:-} ]]; then
  __CTERM_CAPTURE_LOADED=1
  autoload -Uz add-zsh-hook
  __cterm_capture_preexec() {
    cterm capture emit --shell zsh --command "$1" --cwd "$PWD" >/dev/null 2>&1
  }
  add-zsh-hook preexec __cterm_capture_preexec
fi
"#;

const ZSH_RC_BLOCK: &str =
    r#"[ -f "$HOME/.cterm/hooks/zsh-capture.zsh" ] && . "$HOME/.cterm/hooks/zsh-capture.zsh""#;

/// The bash capture source: DEBUG-trap hook, loaded interactively and via
/// `BASH_ENV` for `bash -c` — so it reaches the [`Layer::NonInteractiveHook`].
#[must_use]
pub fn bash() -> Box<dyn CaptureSource> {
    Box::new(ShellHookSource {
        id: "bash-hook",
        title: "bash command hook",
        summary: "DEBUG-trap hook; also catches `bash -c` via BASH_ENV (per-user)",
        shell: "bash",
        rc_file: ".bashrc",
        hook_file: "bash-capture.sh",
        hook_body: BASH_HOOK,
        rc_block: BASH_RC_BLOCK,
        layer: Layer::NonInteractiveHook,
    })
}

/// The zsh capture source: `add-zsh-hook preexec`. Interactive only — zsh has
/// no `-c` hook ([`Layer::InteractiveHook`]).
#[must_use]
pub fn zsh() -> Box<dyn CaptureSource> {
    Box::new(ShellHookSource {
        id: "zsh-hook",
        title: "zsh command hook",
        summary: "add-zsh-hook preexec (interactive sessions only)",
        shell: "zsh",
        rc_file: ".zshrc",
        hook_file: "zsh-capture.zsh",
        hook_body: ZSH_HOOK,
        rc_block: ZSH_RC_BLOCK,
        layer: Layer::InteractiveHook,
    })
}

/// Windows `cmd.exe` Command Processor AutoRun key — runs on every `cmd`,
/// including `cmd /c`.
const CMD_CP_KEY: &str = r"Software\Microsoft\Command Processor";
const CMD_AUTORUN: &str = "AutoRun";

// `\r\n` line endings for a Windows batch file; `%CMDCMDLINE%` is the full
// command line that launched this cmd (e.g. `cmd /c <command>`), so this
// captures the `cmd /c` invocations that bypass the shim.
const CMD_HOOK: &str = "@echo off\r\n\
cterm capture emit --shell cmd --command \"%CMDCMDLINE%\" --cwd \"%CD%\" >nul 2>&1\r\n";

/// The cmd capture source: registers a Command Processor `AutoRun` entry
/// ([`Layer::NonInteractiveHook`]). AutoRun fires on *every* cmd start incl.
/// `cmd /c`, so it catches those bypass invocations; it cannot see individual
/// commands typed in an interactive cmd session (cmd has no per-command hook).
struct CmdAutoRunSource;

impl CmdAutoRunSource {
    fn hook_path(home: &str) -> String {
        format!(r"{home}\.cterm\hooks\cmd-capture.cmd")
    }
}

impl CaptureSource for CmdAutoRunSource {
    fn info(&self) -> SourceInfo {
        SourceInfo {
            id: SourceId::new("cmd-autorun"),
            title: "cmd AutoRun hook".to_owned(),
            summary: "Command Processor AutoRun; catches `cmd /c` via %CMDCMDLINE% (per-user HKCU)"
                .to_owned(),
            platforms: vec![Platform::Windows],
            layer: Layer::NonInteractiveHook,
            privilege: Privilege::User,
        }
    }

    fn availability(&self, probe: &dyn HostProbe) -> Availability {
        match probe.platform() {
            Some(Platform::Windows) if home(probe).is_some() => Availability::Available,
            Some(Platform::Windows) => Availability::Unsupported {
                reason: "USERPROFILE is not set".to_owned(),
            },
            _ => Availability::Unsupported {
                reason: "cmd AutoRun is Windows-only".to_owned(),
            },
        }
    }

    fn enable_plan(&self, probe: &dyn HostProbe) -> EnablePlan {
        let Some(home) = home(probe) else {
            return EnablePlan::default();
        };
        let hook = Self::hook_path(&home);
        EnablePlan::new(vec![
            PlanStep::new(
                "install the cmd AutoRun capture hook",
                Action::WriteFile {
                    path: hook.clone(),
                    contents: CMD_HOOK.to_owned(),
                },
                false,
            ),
            PlanStep::new(
                "register it via Command Processor AutoRun (HKCU)",
                Action::SetRegistryValue {
                    hive: RegistryHive::CurrentUser,
                    key: CMD_CP_KEY.to_owned(),
                    name: CMD_AUTORUN.to_owned(),
                    value: format!("\"{hook}\""),
                },
                false,
            ),
        ])
    }

    fn disable_plan(&self, probe: &dyn HostProbe) -> EnablePlan {
        let Some(home) = home(probe) else {
            return EnablePlan::default();
        };
        EnablePlan::new(vec![
            PlanStep::new(
                "remove the Command Processor AutoRun value",
                Action::DeleteRegistryValue {
                    hive: RegistryHive::CurrentUser,
                    key: CMD_CP_KEY.to_owned(),
                    name: CMD_AUTORUN.to_owned(),
                },
                false,
            ),
            PlanStep::new(
                "remove the cmd AutoRun capture hook",
                Action::RemoveFile {
                    path: Self::hook_path(&home),
                },
                false,
            ),
        ])
    }
}

/// The cmd capture source (Windows AutoRun). See [`CmdAutoRunSource`].
#[must_use]
pub fn cmd() -> Box<dyn CaptureSource> {
    Box::new(CmdAutoRunSource)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    struct FakeProbe {
        platform: Option<Platform>,
        home: Option<String>,
        programs: HashSet<String>,
    }

    impl HostProbe for FakeProbe {
        fn platform(&self) -> Option<Platform> {
            self.platform
        }
        fn is_elevated(&self) -> bool {
            false
        }
        fn env(&self, key: &str) -> Option<String> {
            match key {
                "HOME" => self.home.clone(),
                _ => None,
            }
        }
        fn path_exists(&self, _path: &str) -> bool {
            false
        }
        fn has_program(&self, program: &str) -> bool {
            self.programs.contains(program)
        }
    }

    fn linux_with(shell: &str) -> FakeProbe {
        FakeProbe {
            platform: Some(Platform::Linux),
            home: Some("/home/me".into()),
            programs: HashSet::from([shell.to_owned()]),
        }
    }

    #[test]
    fn bash_available_on_linux_with_bash() {
        assert_eq!(
            bash().availability(&linux_with("bash")),
            Availability::Available
        );
    }

    #[test]
    fn bash_missing_without_bash() {
        let probe = FakeProbe {
            platform: Some(Platform::Linux),
            home: Some("/home/me".into()),
            programs: HashSet::new(),
        };
        assert!(matches!(
            bash().availability(&probe),
            Availability::Missing { .. }
        ));
    }

    #[test]
    fn bash_unsupported_on_windows() {
        let probe = FakeProbe {
            platform: Some(Platform::Windows),
            home: Some(r"C:\Users\me".into()),
            programs: HashSet::from(["bash".into()]),
        };
        assert!(matches!(
            bash().availability(&probe),
            Availability::Unsupported { .. }
        ));
    }

    #[test]
    fn bash_enable_plan_writes_hook_and_rc_without_elevation() {
        let probe = linux_with("bash");
        let plan = bash().enable_plan(&probe);
        assert_eq!(plan.steps.len(), 2);
        assert!(!plan.requires_elevation());
        // First step writes the hook into ~/.cterm/hooks.
        match &plan.steps[0].action {
            Action::WriteFile { path, contents } => {
                assert_eq!(path, "/home/me/.cterm/hooks/bash-capture.sh");
                assert!(contents.contains("trap '__cterm_capture_preexec' DEBUG"));
            }
            other => panic!("unexpected first action: {other:?}"),
        }
        match &plan.steps[1].action {
            Action::AppendOnce { path, marker, .. } => {
                assert_eq!(path, "/home/me/.bashrc");
                assert_eq!(marker, "cterm-capture-bash");
            }
            other => panic!("unexpected second action: {other:?}"),
        }
    }

    #[test]
    fn bash_disable_plan_undoes_enable() {
        let probe = linux_with("bash");
        let plan = bash().disable_plan(&probe);
        assert!(matches!(&plan.steps[0].action, Action::RemoveBlock { .. }));
        assert!(matches!(&plan.steps[1].action, Action::RemoveFile { .. }));
    }

    #[test]
    fn zsh_is_interactive_layer() {
        assert_eq!(zsh().info().layer, Layer::InteractiveHook);
        assert_eq!(bash().info().layer, Layer::NonInteractiveHook);
    }

    fn windows_probe() -> FakeProbe {
        FakeProbe {
            platform: Some(Platform::Windows),
            home: Some(r"C:\Users\me".into()),
            programs: HashSet::new(),
        }
    }

    #[test]
    fn cmd_available_on_windows_and_registers_autorun() {
        let probe = windows_probe();
        assert_eq!(cmd().availability(&probe), Availability::Available);
        let plan = cmd().enable_plan(&probe);
        assert_eq!(plan.steps.len(), 2);
        assert!(!plan.requires_elevation());
        match &plan.steps[1].action {
            Action::SetRegistryValue {
                hive,
                key,
                name,
                value,
            } => {
                assert_eq!(*hive, RegistryHive::CurrentUser);
                assert!(key.ends_with("Command Processor"));
                assert_eq!(name, "AutoRun");
                assert!(value.contains("cmd-capture.cmd"));
            }
            other => panic!("expected registry set, got {other:?}"),
        }
    }

    #[test]
    fn cmd_unsupported_off_windows() {
        let probe = linux_with("cmd");
        assert!(matches!(
            cmd().availability(&probe),
            Availability::Unsupported { .. }
        ));
    }

    #[test]
    fn cmd_disable_removes_registry_and_file() {
        let plan = cmd().disable_plan(&windows_probe());
        assert!(matches!(
            &plan.steps[0].action,
            Action::DeleteRegistryValue { .. }
        ));
        assert!(matches!(&plan.steps[1].action, Action::RemoveFile { .. }));
    }
}
