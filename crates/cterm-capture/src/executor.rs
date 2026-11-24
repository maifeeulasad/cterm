//! Applies an [`EnablePlan`] to the host.
//!
//! The executor performs only the steps that need no elevation (writing a hook
//! file, editing an rc file, an HKCU registry value). Steps marked `elevated`
//! and [`Action::Manual`] steps are **not** run — they're collected into
//! [`ApplyOutcome::deferred`] so the caller can show the operator exactly what
//! still needs root/admin and let them run it themselves. Nothing here silently
//! escalates privilege.

use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

use cterm_core::capture::{Action, EnablePlan, PlanStep, RegistryHive};

/// What happened when a plan was applied.
#[derive(Debug, Default)]
pub struct ApplyOutcome {
    /// Descriptions of steps that were applied.
    pub applied: Vec<String>,
    /// Elevated or manual steps left for the operator to perform.
    pub deferred: Vec<PlanStep>,
    /// Errors from steps that failed, as `"<description>: <error>"`.
    pub errors: Vec<String>,
}

impl ApplyOutcome {
    /// Whether every attempted (non-deferred) step succeeded.
    #[must_use]
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Applies the steps of `plan` the caller is privileged to run: always the
/// unprivileged ones, and the `elevated` ones too when `is_elevated` (the
/// process already has root/admin). Steps it can't run — elevated ones without
/// privilege, and every [`Action::Manual`] — are deferred for the operator.
/// Nothing here escalates on its own.
#[must_use]
pub fn apply(plan: &EnablePlan, is_elevated: bool) -> ApplyOutcome {
    let mut out = ApplyOutcome::default();
    for step in &plan.steps {
        let manual = matches!(step.action, Action::Manual { .. });
        if manual || (step.elevated && !is_elevated) {
            out.deferred.push(step.clone());
            continue;
        }
        match perform(&step.action) {
            Ok(()) => out.applied.push(step.description.clone()),
            Err(e) => out.errors.push(format!("{}: {e}", step.description)),
        }
    }
    out
}

fn perform(action: &Action) -> io::Result<()> {
    match action {
        Action::WriteFile { path, contents } => write_file(path.as_ref(), contents),
        Action::AppendOnce {
            path,
            marker,
            block,
        } => append_once(path.as_ref(), marker, block),
        Action::RemoveFile { path } => remove_file(path.as_ref()),
        Action::RemoveBlock { path, marker } => remove_block(path.as_ref(), marker),
        Action::SetRegistryValue {
            hive,
            key,
            name,
            value,
        } => set_registry(*hive, key, name, value),
        Action::DeleteRegistryValue { hive, key, name } => delete_registry(*hive, key, name),
        Action::RunCommand { program, args } => run_command(program, args),
        // Manual steps are always deferred, never performed here.
        Action::Manual { .. } => Ok(()),
    }
}

fn write_file(path: &Path, contents: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)
}

fn remove_file(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn begin_marker(marker: &str) -> String {
    format!("# >>> {marker} >>>")
}
fn end_marker(marker: &str) -> String {
    format!("# <<< {marker} <<<")
}

/// Appends a marker-guarded block to `path` unless it's already present.
fn append_once(path: &Path, marker: &str, block: &str) -> io::Result<()> {
    let begin = begin_marker(marker);
    let existing = fs::read_to_string(path).unwrap_or_default();
    if existing.contains(&begin) {
        return Ok(()); // already present — idempotent
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut out = existing;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&begin);
    out.push('\n');
    out.push_str(block);
    if !block.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&end_marker(marker));
    out.push('\n');
    fs::write(path, out)
}

/// Removes a marker-guarded block (inclusive of its marker lines) from `path`,
/// leaving everything else untouched. A missing file or block is not an error.
fn remove_block(path: &Path, marker: &str) -> io::Result<()> {
    let Ok(existing) = fs::read_to_string(path) else {
        return Ok(());
    };
    let begin = begin_marker(marker);
    let end = end_marker(marker);
    let mut kept = Vec::new();
    let mut skipping = false;
    for line in existing.lines() {
        if line.trim() == begin {
            skipping = true;
            continue;
        }
        if skipping {
            if line.trim() == end {
                skipping = false;
            }
            continue;
        }
        kept.push(line);
    }
    let mut result = kept.join("\n");
    if !result.is_empty() {
        result.push('\n');
    }
    fs::write(path, result)
}

fn hive_prefix(hive: RegistryHive) -> &'static str {
    match hive {
        RegistryHive::CurrentUser => "HKCU",
        RegistryHive::LocalMachine => "HKLM",
    }
}

fn set_registry(hive: RegistryHive, key: &str, name: &str, value: &str) -> io::Result<()> {
    run_command(
        "reg",
        &[
            "add".into(),
            format!("{}\\{key}", hive_prefix(hive)),
            "/v".into(),
            name.into(),
            "/t".into(),
            "REG_SZ".into(),
            "/d".into(),
            value.into(),
            "/f".into(),
        ],
    )
}

fn delete_registry(hive: RegistryHive, key: &str, name: &str) -> io::Result<()> {
    run_command(
        "reg",
        &[
            "delete".into(),
            format!("{}\\{key}", hive_prefix(hive)),
            "/v".into(),
            name.into(),
            "/f".into(),
        ],
    )
}

fn run_command(program: &str, args: &[String]) -> io::Result<()> {
    let status = Command::new(program).args(args).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("{program} exited with {status}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cterm_core::capture::{Action, EnablePlan, PlanStep};

    /// A throwaway directory under the OS temp dir.
    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "cterm-cap-test-{}-{:?}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
        fn path(&self, name: &str) -> String {
            self.0.join(name).to_string_lossy().into_owned()
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn write_then_remove_file() {
        let dir = TempDir::new();
        let path = dir.path("sub/hook.sh");
        perform(&Action::WriteFile {
            path: path.clone(),
            contents: "hello".into(),
        })
        .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
        perform(&Action::RemoveFile { path: path.clone() }).unwrap();
        assert!(!Path::new(&path).exists());
        // Removing a missing file is fine.
        perform(&Action::RemoveFile { path }).unwrap();
    }

    #[test]
    fn append_once_is_idempotent_and_removable() {
        let dir = TempDir::new();
        let path = dir.path(".bashrc");
        std::fs::write(&path, "export PATH=/x\n").unwrap();
        let append = Action::AppendOnce {
            path: path.clone(),
            marker: "cterm-capture-bash".into(),
            block: ". ~/.cterm/hooks/bash-capture.sh".into(),
        };
        perform(&append).unwrap();
        perform(&append).unwrap(); // second time: no duplicate
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after.matches(">>> cterm-capture-bash >>>").count(), 1);
        assert!(after.contains("export PATH=/x"));
        assert!(after.contains("bash-capture.sh"));

        perform(&Action::RemoveBlock {
            path: path.clone(),
            marker: "cterm-capture-bash".into(),
        })
        .unwrap();
        let cleaned = std::fs::read_to_string(&path).unwrap();
        assert_eq!(cleaned, "export PATH=/x\n");
    }

    #[test]
    fn elevated_and_manual_steps_are_deferred_not_run() {
        let plan = EnablePlan::new(vec![
            PlanStep::new(
                "unprivileged",
                Action::WriteFile {
                    path: TempDir::new().path("f"),
                    contents: "x".into(),
                },
                false,
            ),
            PlanStep::new(
                "needs root",
                Action::RunCommand {
                    program: "auditctl".into(),
                    args: vec![],
                },
                true,
            ),
            PlanStep::new(
                "by hand",
                Action::Manual {
                    instructions: "do it".into(),
                },
                false,
            ),
        ]);
        // Unprivileged caller: the elevated step + the manual are deferred.
        let out = apply(&plan, false);
        assert_eq!(out.applied.len(), 1);
        assert_eq!(out.deferred.len(), 2);
        assert!(out.ok());
    }

    #[test]
    fn elevated_caller_runs_elevated_steps_but_still_defers_manual() {
        let dir = TempDir::new();
        let path = dir.path("elevated-written");
        let plan = EnablePlan::new(vec![
            PlanStep::new(
                "elevated write",
                Action::WriteFile {
                    path: path.clone(),
                    contents: "ok".into(),
                },
                true, // elevated
            ),
            PlanStep::new(
                "by hand",
                Action::Manual {
                    instructions: "do it".into(),
                },
                true,
            ),
        ]);
        // As an elevated caller the elevated write runs; Manual always defers.
        let out = apply(&plan, true);
        assert_eq!(out.applied.len(), 1);
        assert_eq!(out.deferred.len(), 1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "ok");
    }

    #[test]
    fn run_command_failure_is_reported() {
        let out = apply(
            &EnablePlan::new(vec![PlanStep::new(
                "bad command",
                Action::RunCommand {
                    program: "this-program-does-not-exist-xyz".into(),
                    args: vec![],
                },
                false,
            )]),
            false,
        );
        assert!(!out.ok());
        assert_eq!(out.errors.len(), 1);
    }
}
