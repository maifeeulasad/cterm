//! systemd `--user` service management for the daemon (Linux).
//!
//! We use a *user* service, not a system one: it runs as you, needs no root,
//! starts on login, and is torn down with one command — so it can never lock
//! anyone out. The file-writing parts are pure and testable; enabling/disabling
//! shells out to `systemctl --user` (best-effort — it needs a user session bus).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{CliError, CliResult};
use crate::layout::Layout;

/// The systemd unit file name.
pub const UNIT_NAME: &str = "cterm-daemon.service";

/// Default daemon listen address baked into the unit.
pub const DEFAULT_ADDR: &str = "127.0.0.1:50051";

/// `$XDG_CONFIG_HOME/systemd/user` (or `$HOME/.config/systemd/user`).
pub fn default_unit_dir() -> CliResult<PathBuf> {
    let cfg = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))
        .ok_or_else(|| CliError::Msg("neither XDG_CONFIG_HOME nor HOME is set".to_owned()))?;
    Ok(cfg.join("systemd").join("user"))
}

/// Renders the unit file contents.
#[must_use]
pub fn unit_contents(daemon: &Path, db: &Path, addr: &str) -> String {
    format!(
        "[Unit]\n\
         Description=cterm daemon (terminal integration)\n\
         After=default.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={daemon}\n\
         Environment=CTERM_DB={db}\n\
         Environment=CTERM_ADDR={addr}\n\
         Restart=on-failure\n\
         RestartSec=2\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        daemon = daemon.display(),
        db = db.display(),
        addr = addr,
    )
}

/// Locates the `cterm-daemon` binary: next to this executable, or in the shim
/// dir. Returns an error if neither exists.
pub fn locate_daemon(layout: &Layout) -> CliResult<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("cterm-daemon"));
        }
    }
    candidates.push(layout.bin_dir().join("cterm-daemon"));
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| CliError::Msg("could not find the cterm-daemon binary".to_owned()))
}

/// Installs the unit file (copying the daemon into the shim dir for a stable
/// path) and returns the unit file path. Does not call `systemctl`.
pub fn install_unit(
    layout: &Layout,
    unit_dir: &Path,
    daemon_src: &Path,
    addr: &str,
) -> CliResult<PathBuf> {
    fs::create_dir_all(layout.bin_dir())?;
    let daemon_dst = layout.bin_dir().join("cterm-daemon");
    if daemon_src != daemon_dst {
        fs::copy(daemon_src, &daemon_dst)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = fs::metadata(&daemon_dst)?.permissions();
        perm.set_mode(0o755);
        fs::set_permissions(&daemon_dst, perm)?;
    }

    fs::create_dir_all(unit_dir)?;
    let unit_path = unit_dir.join(UNIT_NAME);
    let contents = unit_contents(&daemon_dst, &layout.default_db(), addr);
    fs::write(&unit_path, contents)?;
    Ok(unit_path)
}

/// Removes the unit file. Returns whether a file was removed.
pub fn remove_unit(unit_dir: &Path) -> CliResult<bool> {
    let unit_path = unit_dir.join(UNIT_NAME);
    if unit_path.exists() {
        fs::remove_file(&unit_path)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Runs a `systemctl --user` subcommand, returning combined output. Errors if
/// systemctl is missing or the user bus is unavailable.
pub fn systemctl(args: &[&str]) -> CliResult<String> {
    let output = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()
        .map_err(|e| CliError::Msg(format!("systemctl not available: {e}")))?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    if output.status.success() {
        Ok(text)
    } else {
        Err(CliError::Msg(format!(
            "systemctl --user {} failed: {}",
            args.join(" "),
            text.trim()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_base() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "cterm-svc-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        p
    }

    #[test]
    fn unit_contents_has_execstart_and_env() {
        let unit = unit_contents(
            Path::new("/home/u/.cterm/bin/cterm-daemon"),
            Path::new("/home/u/.cterm/cterm.db"),
            "127.0.0.1:50051",
        );
        assert!(unit.contains("ExecStart=/home/u/.cterm/bin/cterm-daemon"));
        assert!(unit.contains("Environment=CTERM_DB=/home/u/.cterm/cterm.db"));
        assert!(unit.contains("Environment=CTERM_ADDR=127.0.0.1:50051"));
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn install_and_remove_unit_file() {
        let base = temp_base();
        let layout = Layout::new(base.join(".cterm"));
        let unit_dir = base.join("systemd-user");

        // Use any existing file as a stand-in daemon binary.
        let fake_daemon = base.join("cterm-daemon-src");
        fs::create_dir_all(&base).unwrap();
        fs::write(&fake_daemon, b"#!/bin/true\n").unwrap();

        let unit_path = install_unit(&layout, &unit_dir, &fake_daemon, DEFAULT_ADDR).unwrap();
        assert!(unit_path.exists());
        assert!(layout.bin_dir().join("cterm-daemon").exists());
        let contents = fs::read_to_string(&unit_path).unwrap();
        assert!(contents.contains("cterm-daemon"));

        assert!(remove_unit(&unit_dir).unwrap());
        assert!(!unit_path.exists());
        assert!(!remove_unit(&unit_dir).unwrap());

        let _ = fs::remove_dir_all(&base);
    }
}
