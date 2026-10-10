//! On-disk layout of a cterm installation (`~/.cterm` by default).

use std::path::{Path, PathBuf};

use crate::error::{CliError, CliResult};

/// The directory tree cterm installs into. All paths derive from a base dir so
/// the whole thing is relocatable and easy to test against a temp directory.
#[derive(Clone, Debug)]
pub struct Layout {
    base: PathBuf,
}

impl Layout {
    /// A layout rooted at `base`.
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
    }

    /// The default layout at `$HOME/.cterm`.
    pub fn default_home() -> CliResult<Self> {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| CliError::Msg("HOME is not set".to_owned()))?;
        Ok(Self::new(Path::new(&home).join(".cterm")))
    }

    /// The installation root.
    #[must_use]
    pub fn base(&self) -> &Path {
        &self.base
    }

    /// Directory of shim executables, prepended to PATH.
    #[must_use]
    pub fn bin_dir(&self) -> PathBuf {
        self.base.join("bin")
    }

    /// Directory for runtime sockets.
    #[must_use]
    pub fn run_dir(&self) -> PathBuf {
        self.base.join("run")
    }

    /// The JSON config file.
    #[must_use]
    pub fn config_path(&self) -> PathBuf {
        self.base.join("config.json")
    }

    /// Default SQLite database path.
    #[must_use]
    pub fn default_db(&self) -> PathBuf {
        self.base.join("cterm.db")
    }
}
