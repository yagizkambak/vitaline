//! Settings live as JSON in the OS's app config directory.
//! Windows: %APPDATA%\<identifier>\config.json
//! macOS:   ~/Library/Application Support/<identifier>/config.json
//!
//! Tokens are NOT here, they're kept in the keychain (see `secrets`).

use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use tauri::{AppHandle, Manager};

use crate::model::AppConfig;

const FILE: &str = "config.json";

pub fn dir(app: &AppHandle) -> Result<PathBuf> {
    let dir = app.path().app_config_dir()?;
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn path(app: &AppHandle) -> Result<PathBuf> {
    Ok(dir(app)?.join(FILE))
}

/// Reads the settings. Falls back to defaults if the file is missing or
/// corrupt; the app failing to start over a broken settings file would be a
/// bad trade-off.
pub fn load(app: &AppHandle) -> AppConfig {
    let Ok(path) = path(app) else {
        return AppConfig::default();
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return AppConfig::default();
    };
    match serde_json::from_str::<AppConfig>(&text) {
        Ok(cfg) => cfg.sanitized(),
        Err(err) => {
            eprintln!(
                "[vitaline] failed to read {}, using defaults: {err}",
                path.display()
            );
            AppConfig::default()
        }
    }
}

pub fn save(app: &AppHandle, config: &AppConfig) -> Result<()> {
    let path = path(app)?;
    fs::write(&path, serde_json::to_string_pretty(config)?)?;
    Ok(())
}
