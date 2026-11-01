//! Self-serving commands: the CLI managing its own lifecycle
//! (`doctor`, `link`, `update`/`upgrade`, and the `uninstall --all/--purge`
//! helpers). Downloads shell out to `curl`/`tar` so there's no TLS crate to
//! build across platforms; hashing uses the pure-Rust `sha2`.

use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::layout::Layout;
use crate::{commands, service};

/// GitHub `owner/repo` this binary updates from.
pub const REPO: &str = "maifeeulasad/cterm";

/// The platform-null device for discarding output.
#[cfg(windows)]
const NULL_DEVICE: &str = "NUL";
#[cfg(not(windows))]
const NULL_DEVICE: &str = "/dev/null";

/// A binary's file name on this platform (adds `.exe` on Windows).
pub fn bin_name(base: &str) -> String {
    #[cfg(windows)]
    {
        format!("{base}.exe")
    }
    #[cfg(not(windows))]
    {
        base.to_owned()
    }
}

/// Where `cterm link` should place the binaries.
#[derive(Clone, Copy)]
pub enum LinkScope {
    /// A system-wide bin dir (`/usr/local/bin`, or `%ProgramFiles%\cterm`).
    System,
    /// A per-user bin dir (`~/.local/bin`, or `%LOCALAPPDATA%\cterm\bin`).
    User,
}

fn user_bin() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("cterm").join("bin"))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("bin"))
    }
}

fn system_bin() -> PathBuf {
    #[cfg(windows)]
    {
        let base = std::env::var_os("ProgramFiles").unwrap_or_else(|| r"C:\Program Files".into());
        PathBuf::from(base).join("cterm")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/usr/local/bin")
    }
}

fn dir_on_path(dir: &Path) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|e| e == dir))
        .unwrap_or(false)
}

/// Place `src` at `dst`: a symlink on Unix, a copy on Windows (no symlink
/// privilege required). Any existing entry at `dst` is replaced.
fn place(src: &Path, dst: &Path) -> std::io::Result<()> {
    #[cfg(not(windows))]
    {
        let _ = std::fs::remove_file(dst);
        std::os::unix::fs::symlink(src, dst)
    }
    #[cfg(windows)]
    {
        std::fs::copy(src, dst).map(|_| ())
    }
}

/// `cterm link [--system|--user]` — put the `cterm`, `cterm-daemon`, and
/// `cterm-web` binaries sitting next to this executable onto a bin directory.
/// Never edits PATH silently; prints guidance if the target isn't on PATH.
pub fn link(scope: LinkScope) -> i32 {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("cterm: cannot locate this executable: {e}");
            return 1;
        }
    };
    let Some(src_dir) = exe.parent().map(Path::to_path_buf) else {
        eprintln!("cterm: cannot locate the binary directory");
        return 1;
    };

    let target = match scope {
        LinkScope::System => system_bin(),
        LinkScope::User => match user_bin() {
            Some(p) => p,
            None => {
                eprintln!("cterm: cannot determine a user bin dir (HOME/LOCALAPPDATA unset)");
                return 1;
            }
        },
    };

    let system = matches!(scope, LinkScope::System);
    if let Err(e) = std::fs::create_dir_all(&target) {
        eprintln!("cterm: cannot create {}: {e}", target.display());
        if system {
            eprintln!(
                "  (system scope may need elevated privileges — re-run with sudo / as admin)"
            );
        }
        return 1;
    }

    let mut placed = Vec::new();
    for base in ["cterm", "cterm-daemon", "cterm-web"] {
        let name = bin_name(base);
        let src = src_dir.join(&name);
        if !src.exists() {
            continue;
        }
        let dst = target.join(&name);
        if src == dst {
            continue; // already in place
        }
        if let Err(e) = place(&src, &dst) {
            eprintln!(
                "cterm: cannot link {} -> {}: {e}",
                dst.display(),
                src.display()
            );
            if system {
                eprintln!("  (system scope may need elevated privileges)");
            }
            return 1;
        }
        placed.push(name);
    }

    if placed.is_empty() {
        println!(
            "Nothing to link (no cterm binaries found next to {}).",
            exe.display()
        );
        return 0;
    }

    #[cfg(windows)]
    let verb = "Copied";
    #[cfg(not(windows))]
    let verb = "Linked";
    println!("{verb} into {}: {}", target.display(), placed.join(", "));

    if !dir_on_path(&target) {
        println!("\n{} is not on your PATH.", target.display());
        #[cfg(windows)]
        {
            println!("  this session:  $env:PATH += \";{}\"", target.display());
            println!(
                "  to persist:    add it under System -> Environment Variables -> Path (User)"
            );
        }
        #[cfg(not(windows))]
        println!(
            "  export PATH=\"{}:$PATH\"   (add to your shell rc to persist)",
            target.display()
        );
    }
    0
}

/// The latest release version (without a leading `v`), read from the
/// `releases/latest` redirect via `curl`. `None` if curl/network is unavailable.
pub fn latest_version() -> Option<String> {
    let url = format!("https://github.com/{REPO}/releases/latest");
    let out = Command::new("curl")
        .args(["-fsSL", "-o", NULL_DEVICE, "-w", "%{url_effective}", &url])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let effective = String::from_utf8_lossy(&out.stdout);
    let tag = effective.rsplit("/tag/").next()?.trim();
    if tag.is_empty() || tag.contains('/') {
        return None;
    }
    Some(tag.trim_start_matches('v').to_owned())
}

fn parse_ver(s: &str) -> Option<(u32, u32, u32)> {
    let mut it = s.trim_start_matches('v').split('.');
    let a = it.next()?.parse().ok()?;
    let b = it.next()?.parse().ok()?;
    let c = it.next().unwrap_or("0").parse().ok()?;
    Some((a, b, c))
}

/// Whether `latest` is a strictly newer semver than `current`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse_ver(latest), parse_ver(current)), (Some(l), Some(c)) if l > c)
}

fn tcp_reachable(addr: &str) -> bool {
    addr.to_socket_addrs()
        .ok()
        .and_then(|mut addrs| addrs.next())
        .map(|a| TcpStream::connect_timeout(&a, Duration::from_millis(300)).is_ok())
        .unwrap_or(false)
}

fn parent_writable(path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    let probe = parent.join(".cterm-write-probe");
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// `cterm doctor` — read-only self-diagnosis of the whole install.
pub fn doctor(layout: &Layout) -> i32 {
    println!("cterm doctor\n");

    let current = env!("CARGO_PKG_VERSION");
    match latest_version() {
        Some(latest) if is_newer(&latest, current) => {
            println!("version: {current}  (update available: {latest} — run `cterm update`)");
        }
        Some(latest) => println!("version: {current}  (up to date; latest is {latest})"),
        None => println!("version: {current}  (could not check latest — need curl + network)"),
    }

    match std::env::current_exe() {
        Ok(exe) => {
            println!("binary:  {}", exe.display());
            if let Some(dir) = exe.parent() {
                for b in ["cterm-daemon", "cterm-web"] {
                    let present = dir.join(bin_name(b)).exists();
                    println!("  {b}: {}", if present { "found" } else { "MISSING" });
                }
            }
        }
        Err(e) => println!("binary:  (unknown: {e})"),
    }

    match commands::status(layout) {
        Ok(st) => {
            println!(
                "shims:   {}",
                if st.installed_shims.is_empty() {
                    "(none installed)".to_owned()
                } else {
                    st.installed_shims.join(", ")
                }
            );
            println!(
                "         shim dir on PATH: {}",
                if st.on_path { "yes" } else { "no" }
            );
            if let Some(cfg) = st.config {
                let db = Path::new(&cfg.db);
                println!(
                    "records: {}  (exists: {}, writable: {})",
                    cfg.db,
                    db.exists(),
                    parent_writable(db)
                );
            }
        }
        Err(e) => println!("shims:   (error: {e})"),
    }

    let addr = std::env::var("CTERM_ADDR").unwrap_or_else(|_| "127.0.0.1:50051".to_owned());
    println!(
        "daemon:  gRPC {addr} {}",
        if tcp_reachable(&addr) {
            "reachable"
        } else {
            "not reachable (start cterm-daemon)"
        }
    );

    match service::status(layout) {
        Ok(lines) => {
            for l in lines {
                println!("autostart: {l}");
            }
        }
        Err(e) => println!("autostart: (error: {e})"),
    }

    0
}
