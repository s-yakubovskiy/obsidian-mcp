//! Configuration: env/CLI config for vault path, watch toggle, and log level.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub vault_path: PathBuf,
    pub watch: bool,
    pub log_level: String,
}

impl Config {
    /// Load configuration from CLI args and environment variables.
    ///
    /// Priority for vault path: CLI arg (first positional) > `OBSIDIAN_VAULT_PATH` env var.
    pub fn load() -> Result<Self, String> {
        let vault_path = std::env::args()
            .nth(1)
            .or_else(|| std::env::var("OBSIDIAN_VAULT_PATH").ok())
            .map(PathBuf::from)
            .ok_or_else(|| {
                "Vault path required: pass as first argument or set OBSIDIAN_VAULT_PATH".to_string()
            })?;

        let watch = std::env::var("OBSIDIAN_WATCH")
            .unwrap_or_else(|_| "true".into())
            .eq_ignore_ascii_case("true");

        let log_level = std::env::var("OBSIDIAN_LOG_LEVEL").unwrap_or_else(|_| "info".into());

        Ok(Self {
            vault_path,
            watch,
            log_level,
        })
    }
}
