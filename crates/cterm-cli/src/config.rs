//! The persisted CLI config (`~/.cterm/config.json`).

use std::collections::BTreeMap;
use std::fs;

use serde::{Deserialize, Serialize};

use crate::error::CliResult;
use crate::layout::Layout;

/// Installation config: where records live and which shells are wrapped.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// SQLite database path records are written to.
    pub db: String,
    /// Map of wrapped shell name (e.g. `bash`) to the real shell's absolute path.
    pub shells: BTreeMap<String, String>,
}

/// Loads the config from disk.
pub fn load(layout: &Layout) -> CliResult<Config> {
    let data = fs::read(layout.config_path())?;
    Ok(serde_json::from_slice(&data)?)
}

/// Writes the config to disk (creating the base dir).
pub fn save(layout: &Layout, cfg: &Config) -> CliResult<()> {
    fs::create_dir_all(layout.base())?;
    fs::write(layout.config_path(), serde_json::to_vec_pretty(cfg)?)?;
    Ok(())
}
