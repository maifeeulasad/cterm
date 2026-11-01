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

/// The version-less release asset name for the current platform, if supported.
pub fn platform_asset() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("cterm-x86_64-linux.tar.gz"),
        ("macos", "aarch64") => Some("cterm-aarch64-macos.tar.gz"),
        ("macos", "x86_64") => Some("cterm-x86_64-macos.tar.gz"),
        ("windows", "x86_64") => Some("cterm-x86_64-windows.zip"),
        _ => None,
    }
}

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

fn curl_to(url: &str, dest: &Path) -> std::io::Result<()> {
    let Some(dest_s) = dest.to_str() else {
        return Err(std::io::Error::other("non-UTF-8 download path"));
    };
    let status = Command::new("curl")
        .args(["-fsSL", "-o", dest_s, url])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("curl failed for {url}")))
    }
}

fn sha256_hex(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path)?;
    let digest = Sha256::digest(&bytes);
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

/// `cterm update` / `cterm upgrade` — download the latest release for this
/// platform, verify it against the published `.sha256`, and replace the running
/// binary (and the sibling daemon/web binaries) in place. With `check_only`,
/// only report whether an update is available.
pub fn update(check_only: bool) -> i32 {
    let current = env!("CARGO_PKG_VERSION");
    let Some(latest) = latest_version() else {
        eprintln!("cterm: could not determine the latest version (need curl + network).");
        return 1;
    };

    if !is_newer(&latest, current) {
        println!("cterm {current} is up to date (latest is {latest}).");
        return 0;
    }
    println!("Update available: {current} -> {latest}");
    if check_only {
        return 0;
    }

    let Some(asset) = platform_asset() else {
        eprintln!(
            "cterm: no prebuilt release for this platform ({}/{}); build from source or download manually.",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        return 1;
    };

    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("cterm: cannot locate this executable: {e}");
            return 1;
        }
    };

    let work = std::env::temp_dir().join(format!("cterm-update-{}", std::process::id()));
    if let Err(e) = std::fs::create_dir_all(&work) {
        eprintln!("cterm: cannot create temp dir {}: {e}", work.display());
        return 1;
    }
    let rc = update_in(&exe, asset, &work);
    let _ = std::fs::remove_dir_all(&work);
    match rc {
        Ok(()) => {
            println!("Updated to {latest}. Restart any running shims or the daemon to pick it up.");
            0
        }
        Err(e) => {
            eprintln!("cterm: update failed: {e}");
            1
        }
    }
}

/// The download + verify + replace steps, so the caller can always clean up the
/// work dir regardless of outcome.
fn update_in(exe: &Path, asset: &str, work: &Path) -> Result<(), String> {
    let base = format!("https://github.com/{REPO}/releases/latest/download/{asset}");
    let archive = work.join(asset);
    let sums = work.join(format!("{asset}.sha256"));

    println!("Downloading {asset} ...");
    curl_to(&base, &archive).map_err(|e| e.to_string())?;
    curl_to(&format!("{base}.sha256"), &sums).map_err(|e| e.to_string())?;

    // Published checksum file is `<hex>  <name>`; take the first token.
    let published = std::fs::read_to_string(&sums)
        .map_err(|e| e.to_string())?
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase();
    if published.len() != 64 {
        return Err(format!("unexpected checksum in {asset}.sha256"));
    }
    let actual = sha256_hex(&archive).map_err(|e| e.to_string())?;
    if actual != published {
        return Err(format!(
            "checksum mismatch (expected {published}, got {actual}) — refusing to install"
        ));
    }
    println!("Checksum verified.");

    // bsdtar (Windows 10+, macOS) and GNU tar all autodetect gzip; bsdtar also
    // extracts the Windows .zip. So a single `tar -xf` covers every asset.
    let archive_s = archive.to_str().ok_or("non-UTF-8 archive path")?;
    let work_s = work.to_str().ok_or("non-UTF-8 work path")?;
    let status = Command::new("tar")
        .args(["-xf", archive_s, "-C", work_s])
        .status()
        .map_err(|e| format!("could not run tar: {e}"))?;
    if !status.success() {
        return Err("tar failed to extract the archive".into());
    }

    let new_cterm = work.join(bin_name("cterm"));
    if !new_cterm.exists() {
        return Err("extracted archive did not contain cterm".into());
    }

    // Replace the sibling daemon/web binaries first (plain copies; they are not
    // the running process). Then atomically replace this running binary.
    if let Some(dir) = exe.parent() {
        for b in ["cterm-daemon", "cterm-web"] {
            let src = work.join(bin_name(b));
            if !src.exists() {
                continue;
            }
            let dst = dir.join(bin_name(b));
            if let Err(e) = std::fs::copy(&src, &dst) {
                eprintln!(
                    "cterm: warning: could not update {} ({e}); stop it if running and copy manually.",
                    dst.display()
                );
            }
        }
    }

    self_replace::self_replace(&new_cterm).map_err(|e| format!("replacing the running binary: {e}"))
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
