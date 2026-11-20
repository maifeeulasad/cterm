//! `cterm register` — register a cterm shim as a terminal profile.
//!
//! The hybrid flow wants a human to open a *recorded* interactive shell: their
//! terminal launches the cterm shim (`~/.cterm/bin/<shell>`) instead of the raw
//! shell, and from there the daemon records the session and an agent can
//! observe it. This command wires that up.
//!
//! There is no cross-platform API for "add a terminal profile", so the command
//! does the most that each platform allows:
//!
//!   * **Windows** — adds a `cterm (<shell>)` profile directly to Windows
//!     Terminal's `settings.json`. (JSON *fragments* are avoided: on the
//!     packaged/Store build the per-user `Fragments\` folder can break settings
//!     loading entirely — microsoft/terminal#20893.) WT hot-reloads settings, so
//!     the profile appears without a restart. `--print` shows the profile;
//!     `--remove` deletes it.
//!   * **macOS / Linux** — terminal emulators store profiles in their own
//!     formats with no shared drop-in, so the command prints the exact settings
//!     to paste into the emulator's profile UI (name, command, preserve-cwd,
//!     exit-on-exit), generalizing the recipe from issue #26.

use crate::commands::{self, shim_file_name};
use crate::layout::Layout;

/// Parsed `cterm register` options.
pub struct RegisterArgs {
    /// Shells to register (normalized). Empty → every installed shim.
    pub shells: Vec<String>,
    /// Show what would be registered without writing anything.
    pub print: bool,
    /// Remove previously registered profiles instead of adding them.
    pub remove: bool,
}

/// One shell to register: its normalized name and the shim path it launches.
struct Target {
    shell: String,
    shim: std::path::PathBuf,
}

/// Entry point used by `main.rs`.
pub fn register(layout: &Layout, args: RegisterArgs) -> i32 {
    let shells = match resolve_shells(layout, &args.shells) {
        Ok(s) => s,
        Err(code) => return code,
    };

    let mut targets = Vec::new();
    for shell in shells {
        let shim = layout.bin_dir().join(shim_file_name(&shell));
        if !args.remove && !shim.exists() {
            eprintln!(
                "cterm: no shim for '{shell}' at {} — run `cterm install {shell}` first.",
                shim.display()
            );
            return 1;
        }
        targets.push(Target { shell, shim });
    }

    #[cfg(windows)]
    {
        windows_terminal::apply(&targets, args.print, args.remove)
    }
    #[cfg(not(windows))]
    {
        // There's no file to write on non-Windows, so `--print` is implicit —
        // we only ever print. Touch the flag so it isn't flagged "never read".
        let _ = args.print;
        unix_profile::print(&targets, args.remove);
        0
    }
}

/// Resolves the shells to register: the requested ones (each validated as
/// wrappable), or every installed wrappable shim when none are named.
fn resolve_shells(layout: &Layout, requested: &[String]) -> Result<Vec<String>, i32> {
    if !requested.is_empty() {
        let mut out = Vec::new();
        for name in requested {
            let name = commands::normalize_shell_name(name);
            if !commands::is_wrappable(&name) {
                eprintln!("cterm: '{name}' is not a shell cterm can wrap.");
                return Err(2);
            }
            if !out.contains(&name) {
                out.push(name);
            }
        }
        return Ok(out);
    }
    match commands::status(layout) {
        // installed_shims are on-disk file names (e.g. `powershell.exe` on
        // Windows); normalize back to bare shell names and skip non-shell files
        // that may share the bin dir (e.g. the service-staged cterm-daemon
        // launcher). Register every real wrappable shim.
        Ok(st) => {
            let mut out: Vec<String> = Vec::new();
            for s in st.installed_shims {
                let n = commands::normalize_shell_name(&s);
                if commands::is_wrappable(&n) && !out.contains(&n) {
                    out.push(n);
                }
            }
            if out.is_empty() {
                eprintln!("cterm: no shims installed — run `cterm install` first.");
                return Err(1);
            }
            Ok(out)
        }
        Err(e) => {
            eprintln!("cterm: {e}");
            Err(1)
        }
    }
}

/// Quotes a path for a terminal `commandline` string (paths often have spaces).
fn quote(path: &std::path::Path) -> String {
    format!("\"{}\"", path.display())
}

#[cfg(windows)]
mod windows_terminal {
    //! Registers the profile by editing Windows Terminal's `settings.json`.
    //!
    //! We do NOT use JSON *fragments*: on packaged (Store/MSIX) Windows Terminal
    //! the mere existence of the per-user `Fragments\` folder can make WT fail to
    //! load settings and fall back to defaults (microsoft/terminal#20893), and a
    //! valid fragment does not rescue it — so fragments are unusable there.
    //! Editing `settings.json` is reliable, and WT hot-reloads it, so the profile
    //! appears without a restart. We also delete any legacy `Fragments\cterm`
    //! folder an older cterm wrote, to avoid triggering that bug.

    use super::{quote, Target};
    use std::path::{Path, PathBuf};

    use serde_json::Value;

    /// A fixed namespace for deriving stable, per-shim cterm profile GUIDs.
    const CTERM_NS: uuid::Uuid = uuid::Uuid::from_u128(0xc73e6d2a1f4b4c8e9a2d7b3f5e8c1a60);

    /// Candidate `settings.json` paths, most specific first (Store, Store
    /// Preview, then unpackaged).
    fn settings_candidates() -> Vec<PathBuf> {
        let mut v = Vec::new();
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let local = PathBuf::from(local);
            let pkg = |id: &str| {
                local
                    .join("Packages")
                    .join(id)
                    .join("LocalState")
                    .join("settings.json")
            };
            v.push(pkg("Microsoft.WindowsTerminal_8wekyb3d8bbwe"));
            v.push(pkg("Microsoft.WindowsTerminalPreview_8wekyb3d8bbwe"));
            v.push(
                local
                    .join("Microsoft")
                    .join("Windows Terminal")
                    .join("settings.json"),
            );
        }
        v
    }

    fn settings_path() -> Option<PathBuf> {
        settings_candidates().into_iter().find(|p| p.exists())
    }

    /// Stable GUID for this shim's profile (so re-registering updates in place
    /// and `--remove` can find it).
    fn profile_guid(shim: &Path) -> String {
        let id = uuid::Uuid::new_v5(&CTERM_NS, shim.to_string_lossy().as_bytes());
        format!("{{{id}}}")
    }

    fn profile_json(name: &str, shim: &Path, guid: &str) -> Value {
        serde_json::json!({
            "name": name,
            "guid": guid,
            "commandline": quote(shim),
            "hidden": false,
        })
    }

    /// The `profiles.list` array (current shape) or a top-level `profiles` array
    /// (legacy), whichever this settings file uses.
    fn profiles_list(root: &mut Value) -> Option<&mut Vec<Value>> {
        let profiles = root.get_mut("profiles")?;
        if profiles.is_array() {
            return profiles.as_array_mut();
        }
        profiles.get_mut("list")?.as_array_mut()
    }

    /// Delete a legacy `Fragments\cterm` folder (and the empty `Fragments` dir)
    /// a pre-0.8.1 cterm may have written — it risks the settings-load bug.
    fn remove_legacy_fragment() {
        let Some(local) = std::env::var_os("LOCALAPPDATA") else {
            return;
        };
        let frag = PathBuf::from(&local)
            .join("Microsoft")
            .join("Windows Terminal")
            .join("Fragments");
        let _ = std::fs::remove_dir_all(frag.join("cterm"));
        // Only removes Fragments/ if now empty; ignore errors (e.g. VS's own).
        let _ = std::fs::remove_dir(&frag);
    }

    pub fn apply(targets: &[Target], print: bool, remove: bool) -> i32 {
        if print {
            println!("# Profiles cterm would add to Windows Terminal settings.json:");
            for t in targets {
                let name = format!("cterm ({})", t.shell);
                let p = profile_json(&name, &t.shim, &profile_guid(&t.shim));
                println!("{}", serde_json::to_string_pretty(&p).unwrap_or_default());
            }
            return 0;
        }

        remove_legacy_fragment();

        let Some(path) = settings_path() else {
            eprintln!("cterm: could not find a Windows Terminal settings.json (is it installed?).");
            return 1;
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("cterm: cannot read {}: {e}", path.display());
                return 1;
            }
        };
        let mut root: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("cterm: {} is not plain JSON ({e}).", path.display());
                eprintln!(
                    "  If it has // comments, add the profile by hand (see docs/register.md)."
                );
                return 1;
            }
        };
        let Some(list) = profiles_list(&mut root) else {
            eprintln!("cterm: unexpected settings.json shape (no profiles list).");
            return 1;
        };

        // Apply every target against the one profiles list.
        let mut done = Vec::new();
        for t in targets {
            let name = format!("cterm ({})", t.shell);
            let guid = profile_guid(&t.shim);
            let pos = list
                .iter()
                .position(|p| p.get("guid").and_then(Value::as_str) == Some(guid.as_str()));
            if remove {
                if let Some(i) = pos {
                    list.remove(i);
                    done.push(format!("removed \"{name}\""));
                }
            } else {
                let profile = profile_json(&name, &t.shim, &guid);
                match pos {
                    Some(i) => list[i] = profile,
                    None => list.push(profile),
                }
                done.push(format!("\"{name}\" -> {}", t.shim.display()));
            }
        }

        if done.is_empty() {
            println!("No cterm profiles to change in {}.", path.display());
            return 0;
        }

        let verb = if remove { "Removed" } else { "Registered" };
        let mut msg = format!("{verb} {} profile(s) in Windows Terminal:", done.len());
        for d in &done {
            msg.push_str(&format!("\n  {d}"));
        }
        msg.push_str(&format!("\n  settings: {}", path.display()));
        if !remove {
            msg.push_str(
                "\nWindows Terminal hot-reloads settings, so they appear in the new-tab (v) menu now.\
                 \nUndo with: cterm register --remove",
            );
        }
        write_back(&path, &root, &msg)
    }

    fn write_back(path: &Path, root: &Value, success_msg: &str) -> i32 {
        let Ok(s) = serde_json::to_string_pretty(root) else {
            eprintln!("cterm: could not serialize settings.json");
            return 1;
        };
        match std::fs::write(path, s) {
            Ok(()) => {
                println!("{success_msg}");
                0
            }
            Err(e) => {
                eprintln!("cterm: cannot write {}: {e}", path.display());
                1
            }
        }
    }
}

#[cfg(not(windows))]
mod unix_profile {
    use super::{quote, Target};

    /// Prints the settings to paste into a terminal emulator's profile UI, one
    /// block per target shell.
    pub fn print(targets: &[Target], remove: bool) {
        if remove {
            println!("To remove the cterm profile(s), delete these from your terminal emulator's");
            println!("settings (iTerm2, GNOME Terminal, Konsole, …):");
            for t in targets {
                println!("  - \"cterm ({})\"", t.shell);
            }
            return;
        }
        println!("Terminal emulators store profiles in their own formats, so add a profile");
        println!("for each of these (the setting names vary by emulator):\n");
        for t in targets {
            println!("  Profile name          cterm ({})", t.shell);
            println!("  Command / Custom      {}", quote(&t.shim));
            println!(
                "  Working directory     preserve / inherit the current directory (\"Always\")"
            );
            println!("  When command exits    close the terminal\n");
        }
        println!("Opening that profile starts a recorded shell session via the cterm shim.");
        println!("See docs/register.md for per-emulator steps.");
    }
}
