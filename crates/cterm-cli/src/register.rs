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
//!   * **Windows** — writes a [Windows Terminal *fragment*][frag] that adds a
//!     `cterm` profile. Fragments are the supported drop-in mechanism, so this
//!     is a real one-shot registration. `--print` shows the fragment without
//!     writing; `--remove` deletes it.
//!   * **macOS / Linux** — terminal emulators store profiles in their own
//!     formats with no shared drop-in, so the command prints the exact settings
//!     to paste into the emulator's profile UI (name, command, preserve-cwd,
//!     exit-on-exit), generalizing the recipe from issue #26.
//!
//! [frag]: https://learn.microsoft.com/windows/terminal/json-fragment-extensions

use std::path::PathBuf;

use crate::commands::{self, shim_file_name};
use crate::layout::Layout;

/// Parsed `cterm register` options.
pub struct RegisterArgs {
    /// Shell to register (normalized). `None` → the first installed shim.
    pub shell: Option<String>,
    /// Show what would be registered without writing anything.
    pub print: bool,
    /// Remove a previously registered profile instead of adding one.
    pub remove: bool,
}

/// Entry point used by `main.rs`.
pub fn register(layout: &Layout, args: RegisterArgs) -> i32 {
    let shell = match resolve_shell(layout, args.shell.as_deref()) {
        Ok(s) => s,
        Err(code) => return code,
    };

    let shim = layout.bin_dir().join(shim_file_name(&shell));
    if !args.remove && !shim.exists() {
        eprintln!(
            "cterm: no shim for '{shell}' at {} — run `cterm install {shell}` first.",
            shim.display()
        );
        return 1;
    }

    #[cfg(windows)]
    {
        windows_terminal::apply(&shell, &shim, args.print, args.remove)
    }
    #[cfg(not(windows))]
    {
        unix_profile::print(&shell, &shim, args.remove);
        0
    }
}

/// Picks the shell to register: the requested one (validated as wrappable), or
/// the first installed shim when none is named.
fn resolve_shell(layout: &Layout, requested: Option<&str>) -> Result<String, i32> {
    if let Some(name) = requested {
        let name = commands::normalize_shell_name(name);
        if !commands::is_wrappable(&name) {
            eprintln!("cterm: '{name}' is not a shell cterm can wrap.");
            return Err(2);
        }
        return Ok(name);
    }
    match commands::status(layout) {
        // installed_shims are on-disk file names (e.g. `powershell.exe` on
        // Windows); normalize back to a bare shell name.
        Ok(st) => st
            .installed_shims
            .into_iter()
            .next()
            .map(|s| commands::normalize_shell_name(&s))
            .ok_or_else(|| {
                eprintln!("cterm: no shims installed — run `cterm install` first.");
                1
            }),
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
    use super::{quote, PathBuf};
    use std::path::Path;

    /// The Windows Terminal fragments directory for the current user.
    fn fragments_dir() -> Option<PathBuf> {
        std::env::var_os("LOCALAPPDATA").map(|p| {
            PathBuf::from(p)
                .join("Microsoft")
                .join("Windows Terminal")
                .join("Fragments")
                .join("cterm")
        })
    }

    /// The fragment JSON that adds a `cterm (<shell>)` profile launching `shim`.
    /// `startingDirectory: null` tells Windows Terminal to inherit the cwd it is
    /// launched from, matching the "preserve working directory" intent.
    fn fragment(shell: &str, shim: &Path) -> String {
        let value = serde_json::json!({
            "$schema": "https://aka.ms/terminal-profiles-schema",
            "profiles": [{
                "name": format!("cterm ({shell})"),
                "commandline": quote(shim),
                "startingDirectory": serde_json::Value::Null,
            }]
        });
        serde_json::to_string_pretty(&value).unwrap_or_default()
    }

    pub fn apply(shell: &str, shim: &Path, print: bool, remove: bool) -> i32 {
        let Some(dir) = fragments_dir() else {
            eprintln!(
                "cterm: cannot locate the Windows Terminal fragments dir (LOCALAPPDATA unset)."
            );
            return 1;
        };
        let file = dir.join("cterm.json");

        if remove {
            match std::fs::remove_file(&file) {
                Ok(()) => println!(
                    "Removed Windows Terminal profile fragment: {}",
                    file.display()
                ),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    println!("No cterm fragment to remove at {}.", file.display());
                }
                Err(e) => {
                    eprintln!("cterm: could not remove {}: {e}", file.display());
                    return 1;
                }
            }
            return 0;
        }

        let json = fragment(shell, shim);
        if print {
            println!("# Windows Terminal fragment for: {}", file.display());
            println!("{json}");
            return 0;
        }

        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("cterm: could not create {}: {e}", dir.display());
            return 1;
        }
        if let Err(e) = std::fs::write(&file, json) {
            eprintln!("cterm: could not write {}: {e}", file.display());
            return 1;
        }
        println!("Registered a \"cterm ({shell})\" profile in Windows Terminal.");
        println!("  fragment: {}", file.display());
        println!("  launches: {}", shim.display());
        println!(
            "Open a new tab from the \"cterm ({shell})\" profile to start a recorded session."
        );
        println!("Undo with: cterm register --remove");
        0
    }
}

#[cfg(not(windows))]
mod unix_profile {
    use super::quote;
    use std::path::Path;

    /// Prints the settings to paste into a terminal emulator's profile UI.
    pub fn print(shell: &str, shim: &Path, remove: bool) {
        if remove {
            println!("To remove the cterm profile, delete the \"cterm ({shell})\" profile from");
            println!("your terminal emulator's settings (iTerm2, GNOME Terminal, Konsole, …).");
            return;
        }
        println!("Terminal emulators store profiles in their own formats, so add a profile");
        println!("with these settings (the names vary by emulator):\n");
        println!("  Profile name          cterm ({shell})");
        println!("  Command / Custom      {}", quote(shim));
        println!("  Working directory     preserve / inherit the current directory (\"Always\")");
        println!("  When command exits    close the terminal\n");
        println!("Opening that profile starts a recorded shell session via the cterm shim.");
        println!("See docs/register.md for per-emulator steps.");
    }
}
