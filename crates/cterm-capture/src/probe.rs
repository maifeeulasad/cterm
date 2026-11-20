//! [`SystemProbe`]: the live [`HostProbe`] that inspects the real machine.

use std::path::Path;

use cterm_core::capture::HostProbe;
use cterm_core::Platform;

/// A [`HostProbe`] backed by the real environment. Cheap, read-only queries;
/// anything it can't answer safely it answers conservatively (e.g. "not
/// elevated"), so callers never over-estimate capability.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemProbe;

impl HostProbe for SystemProbe {
    fn platform(&self) -> Option<Platform> {
        Platform::current()
    }

    fn is_elevated(&self) -> bool {
        is_elevated()
    }

    fn env(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    fn path_exists(&self, path: &str) -> bool {
        Path::new(path).exists()
    }

    fn has_program(&self, program: &str) -> bool {
        program_on_path(program)
    }
}

/// Searches `PATH` for `program`, trying `PATHEXT` extensions on Windows.
fn program_on_path(program: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    let candidates = executable_names(program);
    std::env::split_paths(&paths).any(|dir| candidates.iter().any(|name| dir.join(name).is_file()))
}

#[cfg(windows)]
fn executable_names(program: &str) -> Vec<String> {
    if Path::new(program).extension().is_some() {
        return vec![program.to_owned()];
    }
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
    let mut names: Vec<String> = pathext
        .split(';')
        .filter(|e| !e.is_empty())
        .map(|e| format!("{program}{}", e.to_lowercase()))
        .collect();
    names.push(program.to_owned());
    names
}

#[cfg(not(windows))]
fn executable_names(program: &str) -> Vec<String> {
    vec![program.to_owned()]
}

#[cfg(unix)]
fn is_elevated() -> bool {
    // `geteuid` is a safe wrapper in `nix`; root is euid 0.
    nix::unistd::geteuid().is_root()
}

#[cfg(windows)]
fn is_elevated() -> bool {
    windows_elevation::is_elevated()
}

/// Scopes the one unavoidable FFI block (querying the process token's elevation)
/// so the rest of the crate stays `deny(unsafe_code)` — same pattern as
/// cterm-cli's `win_console`.
#[cfg(windows)]
mod windows_elevation {
    #![allow(unsafe_code)]

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// Whether the current process token is elevated (running as Administrator).
    /// Returns `false` on any failure — conservative by design.
    pub fn is_elevated() -> bool {
        unsafe {
            let mut token: HANDLE = INVALID_HANDLE_VALUE;
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return false;
            }
            let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
            let mut size = 0u32;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                (&mut elevation as *mut TOKEN_ELEVATION).cast(),
                u32::try_from(std::mem::size_of::<TOKEN_ELEVATION>()).unwrap_or(0),
                &mut size,
            );
            CloseHandle(token);
            ok != 0 && elevation.TokenIsElevated != 0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_this_platform() {
        // On any supported CI host this is Some; the probe just mirrors the OS.
        assert_eq!(SystemProbe.platform(), Platform::current());
    }

    #[test]
    fn finds_a_known_program() {
        // A shell is always present: cmd on Windows, sh elsewhere.
        let shell = if cfg!(windows) { "cmd" } else { "sh" };
        assert!(SystemProbe.has_program(shell), "expected {shell} on PATH");
        assert!(!SystemProbe.has_program("definitely-not-a-real-program-xyz"));
    }

    #[test]
    fn env_and_paths() {
        assert!(SystemProbe.env("PATH").is_some());
        assert!(!SystemProbe.path_exists("/this/path/should/not/exist/xyzzy"));
    }
}
