//! `cterm capture` — configure which command-capture sources are active.
//!
//! Lists the catalog with per-host availability, and enables/disables sources.
//! Enabling applies the unprivileged steps of a source's plan directly; any step
//! that needs elevation is shown and, unless `--yes` was passed, gated behind an
//! accept/reject prompt — cterm never silently escalates. The set of enabled
//! sources is persisted to `<base>/capture.json`.

use std::io::Write;
use std::path::PathBuf;

use cterm_capture::{apply, CaptureConfig, SystemProbe};
use cterm_core::capture::{Availability, CaptureSource, HostProbe, SourceId};

use crate::layout::Layout;

/// Entry point for `cterm capture …`, dispatched from `main`.
pub fn run(layout: &Layout, args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("list") | Some("status") | None => cmd_list(layout),
        Some("enable") => cmd_set(layout, &args[1..], true),
        Some("disable") => cmd_set(layout, &args[1..], false),
        Some("-h") | Some("--help") => {
            print!("{}", usage());
            0
        }
        Some(other) => {
            eprintln!("cterm capture: unknown subcommand '{other}'\n\n{}", usage());
            2
        }
    }
}

fn config_path(layout: &Layout) -> PathBuf {
    layout.base().join("capture.json")
}

fn load_config(layout: &Layout) -> CaptureConfig {
    std::fs::read_to_string(config_path(layout))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_config(layout: &Layout, config: &CaptureConfig) -> std::io::Result<()> {
    std::fs::create_dir_all(layout.base())?;
    let json = serde_json::to_string_pretty(config).unwrap_or_else(|_| "{}".to_owned());
    std::fs::write(config_path(layout), json)
}

fn availability_label(a: &Availability) -> String {
    match a {
        Availability::Available => "available".to_owned(),
        Availability::NeedsElevation { privilege } => format!("needs {}", privilege.describe()),
        Availability::Missing { reason } => format!("unavailable — {reason}"),
        Availability::Unsupported { reason } => format!("n/a — {reason}"),
    }
}

fn cmd_list(layout: &Layout) -> i32 {
    let probe = SystemProbe;
    let config = load_config(layout);
    println!(
        "Command-capture sources (this host: {}):\n",
        probe
            .platform()
            .map_or("unsupported", |_| std::env::consts::OS)
    );
    for source in cterm_capture::catalog() {
        let info = source.info();
        let avail = source.availability(&probe);
        let on = if config.is_enabled(&info.id) {
            "[on] "
        } else {
            "[  ] "
        };
        println!(
            "{on}{:<24} {:<20} {}",
            info.id.as_str(),
            info.layer.label(),
            info.title
        );
        println!("      {}", info.summary);
        println!("      availability: {}", availability_label(&avail));
    }
    println!("\nEnable with:  cterm capture enable <id> [<id>...]");
    0
}

fn cmd_set(layout: &Layout, args: &[String], enable: bool) -> i32 {
    let mut ids: Vec<String> = Vec::new();
    let mut yes = false;
    for a in args {
        match a.as_str() {
            "--yes" | "-y" => yes = true,
            other if other.starts_with('-') => {
                eprintln!("cterm capture: unknown flag '{other}'");
                return 2;
            }
            other => ids.push(other.to_owned()),
        }
    }
    if ids.is_empty() {
        eprintln!(
            "cterm capture {}: name at least one source id (see `cterm capture list`).",
            if enable { "enable" } else { "disable" }
        );
        return 2;
    }

    let probe = SystemProbe;
    let mut config = load_config(layout);
    let mut changed = false;
    let mut failed = false;

    for id_str in ids {
        let id = SourceId::new(id_str.clone());
        let Some(source) = cterm_capture::find(&id) else {
            eprintln!("cterm capture: unknown source '{id_str}' (see `cterm capture list`).");
            failed = true;
            continue;
        };
        if enable {
            if apply_enable(&*source, &probe, &mut config, yes) {
                changed = true;
            } else {
                failed = true;
            }
        } else if apply_disable(&*source, &probe, &mut config) {
            changed = true;
        }
    }

    if changed {
        if let Err(e) = save_config(layout, &config) {
            eprintln!("cterm capture: could not save config: {e}");
            return 1;
        }
    }
    i32::from(failed)
}

/// Returns `true` if the source ended up enabled (unprivileged steps applied).
fn apply_enable(
    source: &dyn CaptureSource,
    probe: &dyn HostProbe,
    config: &mut CaptureConfig,
    yes: bool,
) -> bool {
    let info = source.info();
    let avail = source.availability(probe);
    if !avail.is_enableable() {
        println!("{}: {} — skipped.", info.id, availability_label(&avail));
        return false;
    }

    let plan = source.enable_plan(probe);
    if plan.is_empty() {
        println!("{}: nothing to do.", info.id);
        return false;
    }

    println!("Enabling {} — the following will be applied:", info.id);
    for step in &plan.steps {
        let tag = if step.elevated {
            " (needs elevation)"
        } else {
            ""
        };
        println!("  - {}{tag}", step.description);
    }

    if plan.requires_elevation() && !probe.is_elevated() && !yes {
        let ok = confirm(&format!(
            "{} needs {}. cterm will apply the unprivileged steps and print the elevated \
             ones for you to run yourself. Proceed?",
            info.id,
            info.privilege.describe()
        ));
        if !ok {
            println!("Skipped {}.", info.id);
            return false;
        }
    }

    let outcome = apply(&plan);
    for done in &outcome.applied {
        println!("  applied: {done}");
    }
    for err in &outcome.errors {
        eprintln!("  error:   {err}");
    }
    if !outcome.deferred.is_empty() {
        println!(
            "  These steps need {} — run them yourself:",
            info.privilege.describe()
        );
        for step in &outcome.deferred {
            println!("    * {}", describe_action(step));
        }
    }

    if outcome.ok() {
        config.enable(info.id.clone());
        println!("{} enabled.", info.id);
        true
    } else {
        eprintln!("{}: enable incomplete (see errors above).", info.id);
        false
    }
}

fn apply_disable(
    source: &dyn CaptureSource,
    probe: &dyn HostProbe,
    config: &mut CaptureConfig,
) -> bool {
    let info = source.info();
    let outcome = apply(&source.disable_plan(probe));
    for done in &outcome.applied {
        println!("  reverted: {done}");
    }
    if !outcome.deferred.is_empty() {
        println!(
            "  These steps need {} — run them yourself:",
            info.privilege.describe()
        );
        for step in &outcome.deferred {
            println!("    * {}", describe_action(step));
        }
    }
    let was = config.disable(&info.id);
    println!("{} disabled.", info.id);
    was || outcome.ok()
}

/// A one-line, copy-pasteable rendering of a deferred action for the operator.
fn describe_action(step: &cterm_core::capture::PlanStep) -> String {
    use cterm_core::capture::Action;
    match &step.action {
        Action::RunCommand { program, args } => {
            format!("{program} {}", args.join(" "))
        }
        Action::SetRegistryValue {
            key, name, value, ..
        } => {
            format!("set registry {key}\\{name} = {value}")
        }
        Action::DeleteRegistryValue { key, name, .. } => {
            format!("delete registry {key}\\{name}")
        }
        Action::WriteFile { path, .. } => format!("write {path}"),
        Action::Manual { instructions } => instructions.clone(),
        other => format!("{other:?}"),
    }
}

fn confirm(prompt: &str) -> bool {
    print!("{prompt} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Help for `cterm capture`.
pub fn usage() -> String {
    "cterm capture — configure command-capture sources.

USAGE:
    cterm capture <list|enable|disable> [args]

COMMANDS:
    list                      Show every source with its layer, privilege, and
                              per-host availability, and whether it's enabled.
    enable <id>... [--yes]    Enable sources. Applies the unprivileged steps;
                              elevated steps are shown and gated behind an
                              accept/reject prompt (skip it with --yes).
    disable <id>...           Disable sources (undo what enable did).

Capture sees commands no single hook can: see `cterm capture list` for which
layer each source covers. Enabled sources are stored in <base>/capture.json.
"
    .to_owned()
}
