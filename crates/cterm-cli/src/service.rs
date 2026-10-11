//! Daemon autostart.
//!
//! Linux: a systemd `--user` service (starts on login, no root, one-command
//! removal). Windows: a per-user Startup-folder launcher that starts the daemon
//! hidden at login (no admin, no registry, removed by deleting one file). Both
//! are per-user and fully reversible, so neither can lock anyone out.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{CliError, CliResult};
use crate::layout::Layout;

/// Default daemon listen address baked into the autostart entry.
pub const DEFAULT_ADDR: &str = "127.0.0.1:50051";

/// The daemon binary file name on this platform.
fn daemon_bin_name() -> &'static str {
    #[cfg(windows)]
    {
        "cterm-daemon.exe"
    }
    #[cfg(not(windows))]
    {
        "cterm-daemon"
    }
}

/// Locates the `cterm-daemon` binary: next to this executable, or in the shim
/// dir. Errors if neither exists.
pub fn locate_daemon(layout: &Layout) -> CliResult<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(daemon_bin_name()));
        }
    }
    candidates.push(layout.bin_dir().join(daemon_bin_name()));
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| CliError::Msg("could not find the cterm-daemon binary".to_owned()))
}

/// Copies the daemon into the shim dir (a stable path for autostart) and
/// returns the destination.
fn stage_daemon(layout: &Layout, src: &Path) -> CliResult<PathBuf> {
    fs::create_dir_all(layout.bin_dir())?;
    let dst = layout.bin_dir().join(daemon_bin_name());
    if src != dst {
        fs::copy(src, &dst)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = fs::metadata(&dst)?.permissions();
        perm.set_mode(0o755);
        fs::set_permissions(&dst, perm)?;
    }
    Ok(dst)
}

/// Enables daemon autostart for the current user.
pub fn enable(layout: &Layout) -> CliResult<Vec<String>> {
    platform::enable(layout)
}

/// Disables daemon autostart.
pub fn disable(layout: &Layout) -> CliResult<Vec<String>> {
    platform::disable(layout)
}

/// Reports autostart status.
pub fn status(layout: &Layout) -> CliResult<Vec<String>> {
    platform::status(layout)
}

// ========================= Linux: systemd --user =========================
#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::process::Command;

    pub const UNIT_NAME: &str = "cterm-daemon.service";

    pub fn default_unit_dir() -> CliResult<PathBuf> {
        let cfg = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))
            .ok_or_else(|| CliError::Msg("neither XDG_CONFIG_HOME nor HOME is set".to_owned()))?;
        Ok(cfg.join("systemd").join("user"))
    }

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

    pub fn install_unit(
        layout: &Layout,
        unit_dir: &Path,
        daemon_src: &Path,
        addr: &str,
    ) -> CliResult<PathBuf> {
        let daemon = stage_daemon(layout, daemon_src)?;
        fs::create_dir_all(unit_dir)?;
        let unit_path = unit_dir.join(UNIT_NAME);
        fs::write(
            &unit_path,
            unit_contents(&daemon, &layout.default_db(), addr),
        )?;
        Ok(unit_path)
    }

    pub fn remove_unit(unit_dir: &Path) -> CliResult<bool> {
        let unit_path = unit_dir.join(UNIT_NAME);
        if unit_path.exists() {
            fs::remove_file(&unit_path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn systemctl(args: &[&str]) -> CliResult<String> {
        let out = Command::new("systemctl")
            .arg("--user")
            .args(args)
            .output()
            .map_err(|e| CliError::Msg(format!("systemctl not available: {e}")))?;
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        if out.status.success() {
            Ok(text)
        } else {
            Err(CliError::Msg(format!(
                "systemctl --user {} failed: {}",
                args.join(" "),
                text.trim()
            )))
        }
    }

    pub fn enable(layout: &Layout) -> CliResult<Vec<String>> {
        let daemon = locate_daemon(layout)?;
        let unit_dir = default_unit_dir()?;
        let unit_path = install_unit(layout, &unit_dir, &daemon, DEFAULT_ADDR)?;
        let mut lines = vec![format!("Wrote unit {}", unit_path.display())];
        match systemctl(&["daemon-reload"]).and_then(|_| systemctl(&["enable", "--now", UNIT_NAME]))
        {
            Ok(_) => {
                lines.push(format!("Enabled {UNIT_NAME} (starts on login)."));
                lines.push("Tip: `loginctl enable-linger $USER` to also start it at boot.".into());
            }
            Err(e) => {
                lines.push(format!("Unit written, but activation failed: {e}"));
                lines.push(format!(
                    "Activate in a desktop session: systemctl --user daemon-reload && systemctl --user enable --now {UNIT_NAME}"
                ));
            }
        }
        Ok(lines)
    }

    pub fn disable(layout: &Layout) -> CliResult<Vec<String>> {
        let unit_dir = default_unit_dir()?;
        let _ = systemctl(&["disable", "--now", UNIT_NAME]);
        let _ = layout;
        if remove_unit(&unit_dir)? {
            Ok(vec![format!("Disabled and removed {UNIT_NAME}.")])
        } else {
            Ok(vec!["No unit to remove.".into()])
        }
    }

    pub fn status(layout: &Layout) -> CliResult<Vec<String>> {
        let _ = layout;
        let unit_dir = default_unit_dir()?;
        let unit_path = unit_dir.join(UNIT_NAME);
        let mut lines = vec![format!(
            "unit: {} ({})",
            unit_path.display(),
            if unit_path.exists() {
                "present"
            } else {
                "absent"
            }
        )];
        for q in ["is-enabled", "is-active"] {
            match systemctl(&[q, UNIT_NAME]) {
                Ok(out) => lines.push(format!("{q}: {}", out.trim())),
                Err(e) => lines.push(format!("{q}: {e}")),
            }
        }
        Ok(lines)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

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
            let base = crate::service::tests_support::temp_base();
            let layout = Layout::new(base.join(".cterm"));
            let unit_dir = base.join("systemd-user");
            let fake = base.join("cterm-daemon-src");
            fs::create_dir_all(&base).unwrap();
            fs::write(&fake, b"#!/bin/true\n").unwrap();

            let unit_path = install_unit(&layout, &unit_dir, &fake, DEFAULT_ADDR).unwrap();
            assert!(unit_path.exists());
            assert!(layout.bin_dir().join("cterm-daemon").exists());
            assert!(fs::read_to_string(&unit_path)
                .unwrap()
                .contains("cterm-daemon"));

            assert!(remove_unit(&unit_dir).unwrap());
            assert!(!unit_path.exists());
            assert!(!remove_unit(&unit_dir).unwrap());
            let _ = fs::remove_dir_all(&base);
        }
    }
}

// ========================= Windows: Startup folder =========================
#[cfg(windows)]
mod platform {
    use super::*;

    pub const STARTUP_VBS: &str = "cterm-daemon.vbs";
    pub const LAUNCHER_CMD: &str = "cterm-daemon-start.cmd";

    pub fn default_startup_dir() -> CliResult<PathBuf> {
        let appdata = std::env::var_os("APPDATA")
            .ok_or_else(|| CliError::Msg("APPDATA is not set".to_owned()))?;
        Ok(Path::new(&appdata)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Startup"))
    }

    /// A .cmd that sets the environment and starts the daemon.
    pub fn launcher_contents(daemon: &Path, db: &Path, addr: &str) -> String {
        format!(
            "@echo off\r\nset CTERM_DB={db}\r\nset CTERM_ADDR={addr}\r\n\"{daemon}\"\r\n",
            db = db.display(),
            addr = addr,
            daemon = daemon.display(),
        )
    }

    /// A .vbs that runs the launcher hidden (window style 0), non-blocking.
    pub fn startup_contents(launcher: &Path) -> String {
        format!(
            "CreateObject(\"WScript.Shell\").Run \"\"\"{}\"\"\", 0, False\r\n",
            launcher.display()
        )
    }

    pub fn install_startup(
        layout: &Layout,
        startup_dir: &Path,
        daemon_src: &Path,
        addr: &str,
    ) -> CliResult<PathBuf> {
        let daemon = stage_daemon(layout, daemon_src)?;
        let launcher = layout.bin_dir().join(LAUNCHER_CMD);
        fs::write(
            &launcher,
            launcher_contents(&daemon, &layout.default_db(), addr),
        )?;
        fs::create_dir_all(startup_dir)?;
        let vbs = startup_dir.join(STARTUP_VBS);
        fs::write(&vbs, startup_contents(&launcher))?;
        Ok(vbs)
    }

    pub fn remove_startup(layout: &Layout, startup_dir: &Path) -> CliResult<bool> {
        let _ = fs::remove_file(layout.bin_dir().join(LAUNCHER_CMD));
        let vbs = startup_dir.join(STARTUP_VBS);
        if vbs.exists() {
            fs::remove_file(&vbs)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn enable(layout: &Layout) -> CliResult<Vec<String>> {
        let daemon = locate_daemon(layout)?;
        let startup_dir = default_startup_dir()?;
        let vbs = install_startup(layout, &startup_dir, &daemon, DEFAULT_ADDR)?;
        Ok(vec![
            format!("Installed Startup entry {}", vbs.display()),
            "The daemon will start hidden at your next login.".into(),
        ])
    }

    pub fn disable(layout: &Layout) -> CliResult<Vec<String>> {
        let startup_dir = default_startup_dir()?;
        if remove_startup(layout, &startup_dir)? {
            Ok(vec![format!("Removed Startup entry {STARTUP_VBS}.")])
        } else {
            Ok(vec!["No Startup entry to remove.".into()])
        }
    }

    pub fn status(layout: &Layout) -> CliResult<Vec<String>> {
        let _ = layout;
        let startup_dir = default_startup_dir()?;
        let vbs = startup_dir.join(STARTUP_VBS);
        Ok(vec![format!(
            "startup: {} ({})",
            vbs.display(),
            if vbs.exists() { "present" } else { "absent" }
        )])
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn startup_scripts_have_env_and_hidden_run() {
            let launcher = launcher_contents(
                Path::new(r"C:\u\.cterm\bin\cterm-daemon.exe"),
                Path::new(r"C:\u\.cterm\cterm.db"),
                "127.0.0.1:50051",
            );
            assert!(launcher.contains("set CTERM_DB=C:\\u\\.cterm\\cterm.db"));
            assert!(launcher.contains("set CTERM_ADDR=127.0.0.1:50051"));
            assert!(launcher.contains(r"C:\u\.cterm\bin\cterm-daemon.exe"));

            let vbs = startup_contents(Path::new(r"C:\u\.cterm\bin\cterm-daemon-start.cmd"));
            assert!(vbs.contains(".Run"));
            assert!(vbs.contains(", 0, False")); // hidden, non-blocking
            assert!(vbs.contains("cterm-daemon-start.cmd"));
        }

        #[test]
        fn install_and_remove_startup_entry() {
            let base = crate::service::tests_support::temp_base();
            let layout = Layout::new(base.join(".cterm"));
            let startup_dir = base.join("Startup");
            let fake = base.join("cterm-daemon-src.exe");
            fs::create_dir_all(&base).unwrap();
            fs::write(&fake, b"stub").unwrap();

            let vbs = install_startup(&layout, &startup_dir, &fake, DEFAULT_ADDR).unwrap();
            assert!(vbs.exists());
            assert!(layout.bin_dir().join("cterm-daemon.exe").exists());
            assert!(layout.bin_dir().join(LAUNCHER_CMD).exists());

            assert!(remove_startup(&layout, &startup_dir).unwrap());
            assert!(!vbs.exists());
            assert!(!remove_startup(&layout, &startup_dir).unwrap());
            let _ = fs::remove_dir_all(&base);
        }
    }
}

// ================== Fallback for other platforms ==================
#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    use super::*;

    pub fn enable(_layout: &Layout) -> CliResult<Vec<String>> {
        Err(CliError::Msg(
            "daemon autostart is only supported on Linux and Windows".to_owned(),
        ))
    }
    pub fn disable(_layout: &Layout) -> CliResult<Vec<String>> {
        enable(_layout)
    }
    pub fn status(_layout: &Layout) -> CliResult<Vec<String>> {
        enable(_layout)
    }
}

#[cfg(test)]
mod tests_support {
    use std::path::PathBuf;

    pub fn temp_base() -> PathBuf {
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
}
