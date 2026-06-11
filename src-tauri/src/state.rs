//! Tauri-managed application state.
//!
//! Holds the in-memory copy of the non-secret app config plus the runtime
//! manager. Config reads and writes go through this module so the persisted
//! `config.json` and the in-memory copy cannot drift apart; secrets never
//! pass through here.

use crate::config::AppConfig;
use crate::error::{AppError, AppResult};
use crate::runtime::RuntimeManager;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

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

    pub(crate) fn load_config(&self, app: &AppHandle) -> AppResult<AppConfig> {
        let path = config_path(app)?;
        let config = match fs::read_to_string(&path) {
            Ok(contents) => serde_json::from_str::<AppConfig>(&contents).map_err(|error| {
                AppError::config_io(format!(
                    "Failed to parse app config at {}: {error}",
                    path.display()
                ))
            })?,
            Err(error) if error.kind() == ErrorKind::NotFound => AppConfig::default(),
            Err(error) => {
                return Err(AppError::config_io(format!(
                    "Failed to read app config at {}: {error}",
                    path.display()
                )));
            }
        };

        config.validate()?;
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

        fs::write(&path, contents).map_err(|error| {
            AppError::config_io(format!(
                "Failed to write app config at {}: {error}",
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

fn config_path(app: &AppHandle) -> AppResult<PathBuf> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join(CONFIG_FILE_NAME))
        .map_err(|error| {
            AppError::config_io(format!("Failed to resolve app config directory: {error}"))
        })
}
