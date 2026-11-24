//! Persisted capture configuration: which sources the operator has turned on.
//!
//! This is just the *intent* — the set of enabled source ids. Loading/saving it
//! to disk is the caller's job (it owns the path, via the CLI's layout); this
//! type stays a pure, serializable value so it's trivial to test and to reason
//! about.

use cterm_core::capture::SourceId;
use serde::{Deserialize, Serialize};

/// The operator's capture configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureConfig {
    /// Ids of the sources that are enabled, kept sorted and de-duplicated.
    #[serde(default)]
    pub enabled: Vec<SourceId>,
}

impl CaptureConfig {
    /// Whether `id` is enabled.
    #[must_use]
    pub fn is_enabled(&self, id: &SourceId) -> bool {
        self.enabled.contains(id)
    }

    /// Marks `id` enabled. Returns `true` if this was a change (it wasn't
    /// already enabled).
    pub fn enable(&mut self, id: SourceId) -> bool {
        if self.is_enabled(&id) {
            return false;
        }
        self.enabled.push(id);
        self.enabled.sort();
        true
    }

    /// Marks `id` disabled. Returns `true` if this was a change.
    pub fn disable(&mut self, id: &SourceId) -> bool {
        let before = self.enabled.len();
        self.enabled.retain(|x| x != id);
        self.enabled.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enable_is_idempotent_and_sorted() {
        let mut cfg = CaptureConfig::default();
        assert!(cfg.enable(SourceId::new("zsh-hook")));
        assert!(cfg.enable(SourceId::new("bash-hook")));
        assert!(!cfg.enable(SourceId::new("bash-hook"))); // already on
        assert_eq!(
            cfg.enabled,
            vec![SourceId::new("bash-hook"), SourceId::new("zsh-hook")]
        );
        assert!(cfg.is_enabled(&SourceId::new("zsh-hook")));
    }

    #[test]
    fn disable_reports_change() {
        let mut cfg = CaptureConfig::default();
        cfg.enable(SourceId::new("bash-hook"));
        assert!(cfg.disable(&SourceId::new("bash-hook")));
        assert!(!cfg.disable(&SourceId::new("bash-hook"))); // nothing to do
        assert!(!cfg.is_enabled(&SourceId::new("bash-hook")));
    }

    #[test]
    fn roundtrips_through_json() {
        let mut cfg = CaptureConfig::default();
        cfg.enable(SourceId::new("linux-auditd"));
        let json = serde_json::to_string(&cfg).unwrap();
        assert_eq!(serde_json::from_str::<CaptureConfig>(&json).unwrap(), cfg);
        // Empty/absent `enabled` deserializes to an empty config.
        assert_eq!(
            serde_json::from_str::<CaptureConfig>("{}").unwrap(),
            CaptureConfig::default()
        );
    }
}
