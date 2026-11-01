//! Self-serving commands: the CLI managing its own lifecycle
//! (`doctor`, `link`, `update`/`upgrade`, and the `uninstall --all/--purge`
//! helpers). Downloads shell out to `curl`/`tar` so there's no TLS crate to
//! build across platforms; hashing uses the pure-Rust `sha2`.

use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
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
