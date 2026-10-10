//! The description of *what* is being run.

use serde::{Deserialize, Serialize};

/// A command invocation as the shim observed it.
///
/// This is the shell's view of a command: the raw line the user typed (when
/// available) plus the resolved program/args the shell was about to execute.
/// We keep both because agents care about intent (`raw`) while auditing cares
/// about the resolved form.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    /// The raw command line as entered, e.g. `git push origin main`.
    pub raw: String,
    /// Working directory at invocation time, if known.
    pub cwd: Option<String>,
    /// Name of the shell that hosted this command, e.g. `zsh`, `bash`.
    pub shell: String,
    /// Selected environment variables captured for audit. We never capture the
    /// whole environment by default — secrets live there — only an explicit
    /// allow-list chosen by configuration.
    #[serde(default)]
    pub env: Vec<EnvVar>,
}

impl Command {
    /// Builds a command from a raw line and host shell, with no cwd or env.
    #[must_use]
    pub fn new(raw: impl Into<String>, shell: impl Into<String>) -> Self {
        Self {
            raw: raw.into(),
            cwd: None,
            shell: shell.into(),
            env: Vec::new(),
        }
    }

    /// Sets the working directory.
    #[must_use]
    pub fn with_cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// Appends a captured environment variable.
    #[must_use]
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push(EnvVar {
            key: key.into(),
            value: value.into(),
        });
        self
    }
}

/// A single captured environment variable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvVar {
    /// The variable name, e.g. `TERM`.
    pub key: String,
    /// The variable value.
    pub value: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_sets_fields() {
        let cmd = Command::new("ls -la", "zsh")
            .with_cwd("/home/maifee")
            .with_env("TERM", "xterm-256color");
        assert_eq!(cmd.raw, "ls -la");
        assert_eq!(cmd.shell, "zsh");
        assert_eq!(cmd.cwd.as_deref(), Some("/home/maifee"));
        assert_eq!(cmd.env.len(), 1);
        assert_eq!(cmd.env[0].key, "TERM");
    }
}
