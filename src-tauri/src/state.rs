//! Tauri-managed application state.
//!
//! Holds the in-memory copy of the non-secret app config plus the runtime
//! manager. Config reads and writes go through this module so the persisted
//! `config.json` and the in-memory copy cannot drift apart; secrets never
//! pass through here.

use crate::config::AppConfig;
use crate::error::{AppError, AppResult};
use crate::events::{DiagnosticCategory, DiagnosticUpdate, emit_diagnostic};
use crate::runtime::RuntimeManager;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, Runtime};

const CONFIG_FILE_NAME: &str = "config.json";

pub(crate) struct AppState {
    config: Mutex<AppConfig>,
    pub(crate) runtime: RuntimeManager,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            config: Mutex::new(AppConfig::default()),
            runtime: RuntimeManager::default(),
        }
    }
}

impl AppState {
    pub(crate) fn config(&self) -> AppResult<AppConfig> {
        self.config
            .lock()
            .map(|config| config.clone())
            .map_err(|_| AppError::state("Config state lock was poisoned."))
    }

    pub(crate) fn load_config<R: Runtime>(&self, app: &AppHandle<R>) -> AppResult<AppConfig> {
        let path = config_path(app)?;
        let config = match fs::read_to_string(&path) {
            // A corrupt or invalid config file must not lock the user out of
            // the Settings page (the form only renders with a loaded config),
            // so fall back to defaults and report it; the next save replaces
            // the broken file.
            Ok(contents) => match parse_valid_config(&contents) {
                Ok(config) => config,
                Err(error) => {
                    tracing::warn!(
                        path = %path.display(),
                        error_message = %error,
                        "config file is unusable; defaults loaded"
                    );

                    emit_diagnostic(
                        app,
                        DiagnosticUpdate::error(
                            DiagnosticCategory::Config,
                            "config.defaults_loaded",
                            "Saved settings could not be loaded",
                            format!(
                                "The settings file at {} is unusable: {error} Default settings \
                                 are in use; saving settings replaces the file.",
                                path.display()
                            ),
                        ),
                    );

                    AppConfig::default()
                }
            },
            Err(error) if error.kind() == ErrorKind::NotFound => AppConfig::default(),
            Err(error) => {
                return Err(AppError::config_io(format!(
                    "Failed to read app config at {}: {error}",
                    path.display()
                )));
            }
        };

        self.replace_config(config.clone())?;

        Ok(config)
    }

    pub(crate) fn save_config(&self, app: &AppHandle, config: AppConfig) -> AppResult<AppConfig> {
        config.validate()?;
        let path = config_path(app)?;
        let parent = path
            .parent()
            .ok_or_else(|| AppError::config_io("App config path has no parent directory."))?;

        fs::create_dir_all(parent).map_err(|error| {
            AppError::config_io(format!(
                "Failed to create app config directory at {}: {error}",
                parent.display()
            ))
        })?;

        let contents = serde_json::to_string_pretty(&config)
            .map_err(|error| AppError::config_io(format!("Failed to serialize config: {error}")))?;

        // Write-then-rename keeps the existing config intact if the app dies
        // mid-write; a torn config.json would otherwise hit load_config's
        // defaults fallback and silently shelve the user's settings.
        let temp_path = path.with_extension("json.tmp");

        fs::write(&temp_path, contents).map_err(|error| {
            AppError::config_io(format!(
                "Failed to write app config at {}: {error}",
                temp_path.display()
            ))
        })?;
        fs::rename(&temp_path, &path).map_err(|error| {
            AppError::config_io(format!(
                "Failed to replace app config at {}: {error}",
                path.display()
            ))
        })?;

        self.replace_config(config.clone())?;

        Ok(config)
    }

    fn replace_config(&self, config: AppConfig) -> AppResult<()> {
        let mut guard = self
            .config
            .lock()
            .map_err(|_| AppError::state("Config state lock was poisoned."))?;

        *guard = config;

        Ok(())
    }
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
    let config = serde_json::from_str::<AppConfig>(contents)
        .map_err(|error| AppError::config_io(format!("Failed to parse app config: {error}.")))?;

    config.validate()?;

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_passes_validation() -> AppResult<()> {
        AppConfig::default().validate()
    }

    #[test]
    fn parse_valid_config_fills_missing_fields_with_defaults() -> AppResult<()> {
        let config = parse_valid_config(r#"{"stt":{"language":"ja"}}"#)?;

        assert_eq!(config.stt.language, "ja");
        assert!(!config.stt.model.is_empty());

        Ok(())
    }

    #[test]
    fn parse_valid_config_preserves_runtime_settings() -> AppResult<()> {
        let config = parse_valid_config(
            r#"{"audio":{"inputDeviceId":"saved-device"},"osc":{"enabled":false}}"#,
        )?;

        assert_eq!(
            config.audio.input_device_id.as_deref(),
            Some("saved-device")
        );
        assert!(!config.osc.enabled);

        Ok(())
    }

    #[test]
    fn parse_valid_config_rejects_malformed_json() {
        assert!(parse_valid_config("{ not json").is_err());
    }

    #[test]
    fn parse_valid_config_rejects_invalid_settings() {
        assert!(parse_valid_config(r#"{"stt":{"language":"  "}}"#).is_err());
    }
}
