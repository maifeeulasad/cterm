//! # cterm-capture
//!
//! Adapters for the command-capture ports defined in
//! [`cterm_core::capture`]. This crate holds:
//!
//!   * [`SystemProbe`] — the live [`cterm_core::capture::HostProbe`] that reads
//!     the real machine (platform, elevation, env, paths, programs).
//!   * the **source catalog** ([`catalog`]) — one
//!     [`cterm_core::capture::CaptureSource`] per known capture mechanism
//!     (shell hooks today; system-level exec auditors to follow).
//!
//! The split keeps all OS-specific detection and install logic here, behind the
//! domain's ports, so the CLI and daemon stay portable. Everything a source
//! does is declarative (an [`cterm_core::capture::EnablePlan`]), so the config
//! flow can show the operator exactly what will change — and flag elevation —
//! before anything happens.

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod probe;
pub mod sources;

pub use probe::SystemProbe;

use cterm_core::capture::{CaptureSource, Platform, SourceId};

/// Every capture source cterm knows about, regardless of platform. Callers
/// filter by [`cterm_core::capture::CaptureSource::availability`] /
/// [`for_platform`] to find the ones usable here.
#[must_use]
pub fn catalog() -> Vec<Box<dyn CaptureSource>> {
    vec![
        // Shell-hook layer (unprivileged, as-typed).
        sources::bash(),
        sources::zsh(),
        sources::cmd(),
        // System-audit layer (privileged, bypass-proof).
        sources::linux_auditd(),
        sources::windows_process_audit(),
        sources::macos_endpoint_security(),
    ]
}

/// The catalog sources that apply to `platform`.
#[must_use]
pub fn for_platform(platform: Platform) -> Vec<Box<dyn CaptureSource>> {
    catalog()
        .into_iter()
        .filter(|s| s.info().supports(platform))
        .collect()
}

/// Looks up a source by id.
#[must_use]
pub fn find(id: &SourceId) -> Option<Box<dyn CaptureSource>> {
    catalog().into_iter().find(|s| &s.info().id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_ids_are_unique_and_findable() {
        let cat = catalog();
        let mut ids: Vec<_> = cat.iter().map(|s| s.info().id).collect();
        ids.sort();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "source ids must be unique");

        assert!(find(&SourceId::new("bash-hook")).is_some());
        assert!(find(&SourceId::new("nope")).is_none());
    }

    #[test]
    fn each_platform_has_shell_hooks_and_a_system_auditor() {
        use cterm_core::capture::Layer;
        for (platform, expected) in [
            (Platform::Linux, 3),   // bash, zsh, auditd
            (Platform::Macos, 3),   // bash, zsh, endpoint-security
            (Platform::Windows, 2), // cmd, process-audit
        ] {
            let sources = for_platform(platform);
            assert_eq!(sources.len(), expected, "{platform:?}");
            assert!(
                sources.iter().any(|s| s.info().layer == Layer::SystemAudit),
                "{platform:?} needs a bypass-proof system auditor"
            );
        }
    }

    #[test]
    fn system_auditors_are_registered() {
        // On a non-matching platform they are Unsupported; the per-OS behavior
        // (NeedsElevation vs Available-when-root) is covered in sources tests.
        assert!(find(&SourceId::new("linux-auditd")).is_some());
        assert!(find(&SourceId::new("windows-process-audit")).is_some());
        assert!(find(&SourceId::new("macos-endpoint-security")).is_some());
    }
}
