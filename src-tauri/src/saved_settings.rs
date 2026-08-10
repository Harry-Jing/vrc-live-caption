//! Persistent non-secret app settings.
//!
//! This module owns the `config.json` path, strict current-schema decoding,
//! and write-then-rename persistence. Invalid saved settings fall back to
//! editable defaults while carrying an explicit review requirement back to
//! the desktop state; secrets never enter this module.

use crate::config::AppConfig;
use crate::error::{AppError, AppResult};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, Runtime};

const CONFIG_FILE_NAME: &str = "config.json";

pub(crate) enum SavedSettingsLoad {
    Ready(AppConfig),
    DefaultsRequireReview {
        config: AppConfig,
        path: PathBuf,
        error: AppError,
    },
}

pub(crate) fn load<R: Runtime>(app: &AppHandle<R>) -> AppResult<SavedSettingsLoad> {
    let path = config_path(app)?;
    load_from_path(path)
}

fn load_from_path(path: PathBuf) -> AppResult<SavedSettingsLoad> {
    match fs::read_to_string(&path) {
        Ok(contents) => match parse_valid_config(&contents) {
            Ok(config) => Ok(SavedSettingsLoad::Ready(config)),
            Err(error) => Ok(SavedSettingsLoad::DefaultsRequireReview {
                config: AppConfig::default(),
                path,
                error,
            }),
        },
        Err(error) if error.kind() == ErrorKind::NotFound => {
            Ok(SavedSettingsLoad::Ready(AppConfig::default()))
        }
        Err(error) => Err(AppError::config_io(format!(
            "Failed to read app config at {}: {error}",
            path.display()
        ))),
    }
}

pub(crate) fn save<R: Runtime>(app: &AppHandle<R>, config: &AppConfig) -> AppResult<()> {
    let path = config_path(app)?;
    save_to_path(&path, config)
}

fn save_to_path(path: &Path, config: &AppConfig) -> AppResult<()> {
    config.validate()?;
    let parent = path
        .parent()
        .ok_or_else(|| AppError::config_io("App config path has no parent directory."))?;

    fs::create_dir_all(parent).map_err(|error| {
        AppError::config_io(format!(
            "Failed to create app config directory at {}: {error}",
            parent.display()
        ))
    })?;

    let contents = serde_json::to_string_pretty(config)
        .map_err(|error| AppError::config_io(format!("Failed to serialize config: {error}")))?;

    // Write-then-rename keeps the existing config intact if the app dies
    // mid-write; a torn config.json would otherwise load editable defaults and
    // silently shelve the user's settings.
    let temp_path = path.with_extension("json.tmp");

    fs::write(&temp_path, contents).map_err(|error| {
        AppError::config_io(format!(
            "Failed to write app config at {}: {error}",
            temp_path.display()
        ))
    })?;
    fs::rename(&temp_path, path).map_err(|error| {
        AppError::config_io(format!(
            "Failed to replace app config at {}: {error}",
            path.display()
        ))
    })?;

    Ok(())
}

fn config_path<R: Runtime>(app: &AppHandle<R>) -> AppResult<PathBuf> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join(CONFIG_FILE_NAME))
        .map_err(|error| {
            AppError::config_io(format!("Failed to resolve app config directory: {error}"))
        })
}

fn parse_valid_config(contents: &str) -> AppResult<AppConfig> {
    let mut value = serde_json::from_str::<serde_json::Value>(contents)
        .map_err(|error| AppError::config_io(format!("Failed to parse app config: {error}.")))?;
    if let Some(osc) = value
        .get_mut("osc")
        .and_then(serde_json::Value::as_object_mut)
    {
        // This pacing knob was removed by ADR 0015. Ignoring only this known
        // field preserves unrelated settings without restoring a configurable
        // rate or weakening strict model/config decoding.
        osc.remove("minIntervalMs");
    }
    let config = serde_json::from_value::<AppConfig>(value)
        .map_err(|error| AppError::config_io(format!("Failed to parse app config: {error}.")))?;

    config.validate()?;

    Ok(config)
}

#[cfg(test)]
#[path = "saved_settings_tests.rs"]
mod tests;
