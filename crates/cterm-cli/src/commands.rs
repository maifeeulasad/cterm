//! `install` / `uninstall` / `status` and shell discovery.
//!
//! The install model is the reversible PATH-shim: a per-user `~/.cterm/bin`
//! holds a copy of this binary named after each wrapped shell. When something
//! on `PATH` runs `bash`, our copy runs instead, wraps the *real* shell (whose
//! absolute path we record at install time, so there is never a self-exec
//! loop), and reports to the daemon. Uninstalling just removes that directory.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{self, Config};
use crate::error::{CliError, CliResult};
use crate::layout::Layout;

/// Shell basenames we are willing to wrap.
#[cfg(windows)]
pub const WRAPPABLE: &[&str] = &["cmd", "powershell", "pwsh", "bash"];
#[cfg(not(windows))]
pub const WRAPPABLE: &[&str] = &["bash", "zsh", "sh", "dash", "fish", "ksh"];

/// Shells installed by default when none are named.
#[cfg(windows)]
const DEFAULT_SHELLS: &[&str] = &["powershell", "cmd"];
#[cfg(not(windows))]
const DEFAULT_SHELLS: &[&str] = &["zsh", "bash"];

/// Normalizes an invoked program name to a bare, lowercase shell name:
/// strips any directory and (on Windows) a trailing `.exe`.
#[must_use]
pub fn normalize_shell_name(arg0: &str) -> String {
    let base = arg0.rsplit(['/', '\\']).next().unwrap_or(arg0);
    let base = base
        .strip_suffix(".exe")
        .or_else(|| base.strip_suffix(".EXE"))
        .unwrap_or(base);
    base.to_lowercase()
}

/// Whether a (possibly path/`.exe`-qualified) name is one we wrap.
#[must_use]
pub fn is_wrappable(name: &str) -> bool {
    WRAPPABLE.contains(&normalize_shell_name(name).as_str())
}

/// The on-disk file name for a shim wrapping `name` (adds `.exe` on Windows).
#[must_use]
pub fn shim_file_name(name: &str) -> String {
    #[cfg(windows)]
    {
        format!("{name}.exe")
    }
    #[cfg(not(windows))]
    {
        name.to_owned()
    }
}

/// Candidate on-disk file names for an executable `name` (expands Windows
/// PATHEXT extensions when `name` has none).
fn executable_names(name: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        if Path::new(name).extension().is_some() {
            return vec![name.to_owned()];
        }
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
        let mut names: Vec<String> = pathext
            .split(';')
            .filter(|e| !e.is_empty())
            .map(|e| format!("{name}{}", e.to_lowercase()))
            .collect();
        names.push(name.to_owned());
        names
    }
    #[cfg(not(windows))]
    {
        vec![name.to_owned()]
    }
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Finds the real shell binary named `name`, searching `PATH` and common
/// locations but never `exclude` (our own shim directory).
#[must_use]
pub fn find_real_shell(name: &str, exclude: &Path) -> Option<PathBuf> {
    let candidates = executable_names(name);
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            if dir == exclude {
                continue;
            }
            for cand_name in &candidates {
                let cand = dir.join(cand_name);
                if is_executable(&cand) {
                    return Some(cand);
                }
            }
        }
    }
    for dir in ["/usr/bin", "/bin", "/usr/local/bin"] {
        for cand_name in &candidates {
            let cand = Path::new(dir).join(cand_name);
            if cand.parent() != Some(exclude) && is_executable(&cand) {
                return Some(cand);
            }
        }
    }
    None
}

#[cfg(unix)]
fn set_executable(path: &Path) -> CliResult<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perm = fs::metadata(path)?.permissions();
    perm.set_mode(0o755);
    fs::set_permissions(path, perm)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> CliResult<()> {
    Ok(())
}

/// Result of an install.
#[derive(Debug)]
pub struct Installed {
    /// The config written.
    pub config: Config,
    /// Shell names that were installed as shims.
    pub shims: Vec<String>,
}

/// Installs shims for `requested` shells (or the defaults that exist, if empty).
pub fn install(layout: &Layout, requested: &[String]) -> CliResult<Installed> {
    fs::create_dir_all(layout.bin_dir())?;
    fs::create_dir_all(layout.run_dir())?;

    let names: Vec<String> = if requested.is_empty() {
        DEFAULT_SHELLS
            .iter()
            .filter(|n| find_real_shell(n, &layout.bin_dir()).is_some())
            .map(|n| (*n).to_owned())
            .collect()
    } else {
        requested.iter().map(|n| normalize_shell_name(n)).collect()
    };
    if names.is_empty() {
        return Err(CliError::Msg(format!(
            "no shells found to wrap (tried: {})",
            DEFAULT_SHELLS.join(", ")
        )));
    }

    let exe = std::env::current_exe()?;
    let mut cfg = config::load(layout).unwrap_or_default();
    if cfg.db.is_empty() {
        cfg.db = layout.default_db().display().to_string();
    }

    let mut shims = Vec::new();
    for name in &names {
        let real = find_real_shell(name, &layout.bin_dir()).ok_or_else(|| {
            CliError::Msg(format!("could not find a real '{name}' on PATH to wrap"))
        })?;
        let dest = layout.bin_dir().join(shim_file_name(name));
        if dest != exe {
            fs::copy(&exe, &dest)?;
        }
        set_executable(&dest)?;
        cfg.shells.insert(name.clone(), real.display().to_string());
        shims.push(name.clone());
    }

    config::save(layout, &cfg)?;
    Ok(Installed { config: cfg, shims })
}

/// Removes the shim directory (data — config and db — is kept). Returns the
/// shim names that were removed.
pub fn uninstall(layout: &Layout) -> CliResult<Vec<String>> {
    let bin = layout.bin_dir();
    let mut removed = Vec::new();
    if bin.exists() {
        for entry in fs::read_dir(&bin)? {
            if let Some(name) = entry?.file_name().to_str() {
                removed.push(name.to_owned());
            }
        }
        fs::remove_dir_all(&bin)?;
    }
    removed.sort();
    Ok(removed)
}

/// A snapshot of install state.
#[derive(Debug)]
pub struct Status {
    /// Install root.
    pub base: PathBuf,
    /// Installed shim names.
    pub installed_shims: Vec<String>,
    /// Loaded config, if present.
    pub config: Option<Config>,
    /// Whether the shim dir is on the current `PATH`.
    pub on_path: bool,
}

/// Reports install state.
pub fn status(layout: &Layout) -> CliResult<Status> {
    let mut installed_shims = Vec::new();
    if layout.bin_dir().exists() {
        for entry in fs::read_dir(layout.bin_dir())? {
            if let Some(name) = entry?.file_name().to_str() {
                installed_shims.push(name.to_owned());
            }
        }
    }
    installed_shims.sort();

    let config = config::load(layout).ok();
    let on_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d == layout.bin_dir()))
        .unwrap_or(false);

    Ok(Status {
        base: layout.base().to_path_buf(),
        installed_shims,
        config,
        on_path,
    })
}

/// The line to add to a shell rc file to put the shims first on `PATH`.
#[must_use]
pub fn path_export_line(layout: &Layout) -> String {
    format!("export PATH=\"{}:$PATH\"", layout.bin_dir().display())
}

/// Appends the PATH export to `rc`, guarded by markers. Returns `false` if the
/// marker was already present (idempotent).
pub fn append_rc(rc: &Path, layout: &Layout) -> CliResult<bool> {
    const BEGIN: &str = "# >>> cterm >>>";
    const END: &str = "# <<< cterm <<<";
    let existing = fs::read_to_string(rc).unwrap_or_default();
    if existing.contains(BEGIN) {
        return Ok(false);
    }
    use std::io::Write;
    let block = format!("\n{BEGIN}\n{}\n{END}\n", path_export_line(layout));
    let mut f = fs::OpenOptions::new().create(true).append(true).open(rc)?;
    f.write_all(block.as_bytes())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_base() -> PathBuf {
        let mut p = std::env::temp_dir();
        let unique = format!(
            "cterm-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        p.push(unique);
        p
    }

    #[test]
    fn install_then_status_then_uninstall() {
        let base = temp_base();
        let layout = Layout::new(&base);

        // A shell that exists on each platform.
        #[cfg(windows)]
        let shell = "cmd";
        #[cfg(not(windows))]
        let shell = "bash";
        let shim_name = shim_file_name(shell);

        let installed = install(&layout, &[shell.to_owned()]).expect("install");
        assert_eq!(installed.shims, vec![shell.to_owned()]);

        // A shim binary exists, and the recorded real path is a different,
        // genuine executable (no self-exec loop).
        let shim = layout.bin_dir().join(&shim_name);
        assert!(is_executable(&shim));
        let real = PathBuf::from(&installed.config.shells[shell]);
        assert_ne!(real, shim);
        assert!(is_executable(&real));
        assert!(!installed.config.db.is_empty());

        // Config round-trips from disk.
        let loaded = config::load(&layout).expect("load config");
        assert_eq!(loaded, installed.config);

        // Status reflects the install.
        let st = status(&layout).expect("status");
        assert!(st.installed_shims.contains(&shim_name));
        assert!(st.config.is_some());

        // Uninstall removes the shim dir but keeps config.
        let removed = uninstall(&layout).expect("uninstall");
        assert!(removed.contains(&shim_name));
        assert!(!shim.exists());
        assert!(layout.config_path().exists());

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn rc_append_is_idempotent() {
        let base = temp_base();
        let layout = Layout::new(&base);
        fs::create_dir_all(layout.base()).unwrap();
        let rc = layout.base().join(".bashrc");

        assert!(append_rc(&rc, &layout).expect("first append"));
        assert!(!append_rc(&rc, &layout).expect("second append"));
        let contents = fs::read_to_string(&rc).unwrap();
        assert_eq!(contents.matches("# >>> cterm >>>").count(), 1);
        assert!(contents.contains(&path_export_line(&layout)));

        let _ = fs::remove_dir_all(&base);
    }
}
