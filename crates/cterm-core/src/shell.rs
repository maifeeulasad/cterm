//! How a bare command string is run on a host.
//!
//! When a caller asks the daemon to `Run` a command but doesn't name a program
//! and argv, the daemon has to pick a shell — and that choice is OS-specific.
//! Rather than scatter `cfg!(windows)` branches through the daemon, we model the
//! choice as a small port ([`CommandShell`]) with one concrete type per
//! platform and a [`host_default`] that returns the right one for the current
//! OS. New platforms (or a configurable override later) slot in as another
//! implementation without touching the call sites.

/// A shell that knows how to run a single command string non-interactively and
/// then exit, reporting the command's own exit status.
pub trait CommandShell: Send + Sync {
    /// The program to execute, e.g. `/bin/bash` or `cmd`.
    fn program(&self) -> String;

    /// The argument vector that runs `command` and exits — e.g.
    /// `["-lc", command]` for a POSIX login shell, `["/C", command]` for `cmd`.
    fn command_args(&self, command: &str) -> Vec<String>;

    /// A short, stable name for this shell, used for the audit record's
    /// `Command.shell` field, e.g. `bash` or `cmd`.
    fn name(&self) -> &'static str;
}

/// A POSIX login shell: `/bin/bash -lc <command>`. Used on Linux and macOS,
/// where `/bin/bash` is always present and `-lc` gives the command a login
/// environment.
#[derive(Clone, Copy, Debug, Default)]
pub struct PosixShell;

impl CommandShell for PosixShell {
    fn program(&self) -> String {
        "/bin/bash".to_owned()
    }

    fn command_args(&self, command: &str) -> Vec<String> {
        vec!["-lc".to_owned(), command.to_owned()]
    }

    fn name(&self) -> &'static str {
        "bash"
    }
}

/// The Windows command processor: `cmd /C <command>`. `cmd.exe` is present on
/// every Windows install and reports the command's `errorlevel` as the process
/// exit code, which keeps `Run`'s status faithful. Callers who want PowerShell
/// pass an explicit program/args instead.
#[derive(Clone, Copy, Debug, Default)]
pub struct CmdShell;

impl CommandShell for CmdShell {
    fn program(&self) -> String {
        "cmd".to_owned()
    }

    fn command_args(&self, command: &str) -> Vec<String> {
        vec!["/C".to_owned(), command.to_owned()]
    }

    fn name(&self) -> &'static str {
        "cmd"
    }
}

/// The default command shell for the host OS: [`CmdShell`] on Windows, otherwise
/// [`PosixShell`].
#[must_use]
pub fn host_default() -> Box<dyn CommandShell> {
    #[cfg(windows)]
    {
        Box::new(CmdShell)
    }
    #[cfg(not(windows))]
    {
        Box::new(PosixShell)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_runs_under_bash_lc() {
        let sh = PosixShell;
        assert_eq!(sh.program(), "/bin/bash");
        assert_eq!(sh.command_args("echo hi"), vec!["-lc", "echo hi"]);
        assert_eq!(sh.name(), "bash");
    }

    #[test]
    fn cmd_runs_under_slash_c() {
        let sh = CmdShell;
        assert_eq!(sh.program(), "cmd");
        assert_eq!(sh.command_args("echo hi"), vec!["/C", "echo hi"]);
        assert_eq!(sh.name(), "cmd");
    }

    #[test]
    fn host_default_matches_the_target_os() {
        let sh = host_default();
        if cfg!(windows) {
            assert_eq!(sh.name(), "cmd");
        } else {
            assert_eq!(sh.name(), "bash");
        }
    }
}
