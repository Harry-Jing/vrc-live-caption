//! Non-secret app configuration shared by Tauri commands and the runtime.
//!
//! This module intentionally stores only ordinary settings and non-sensitive
//! metadata. Service-credential secrets must come from the environment or the
//! system credential store, never this config file.

use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};

pub(crate) const APP_CONFIG_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AppConfig {
    pub(crate) schema_version: u32,
    pub(crate) audio: AudioConfig,
    pub(crate) recognition: RecognitionConfig,
    pub(crate) osc: OscConfig,
    pub(crate) publication: PublicationConfig,
    pub(crate) ui: UiConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: APP_CONFIG_SCHEMA_VERSION,
            audio: AudioConfig::default(),
            recognition: RecognitionConfig::default(),
            osc: OscConfig::default(),
            publication: PublicationConfig::default(),
            ui: UiConfig::default(),
        }
    }
}

impl AppConfig {
    pub(crate) fn validate(&self) -> AppResult<()> {
        if self.schema_version != APP_CONFIG_SCHEMA_VERSION {
            return Err(AppError::config(format!(
                "Unsupported config schema version {}. Expected {}.",
                self.schema_version, APP_CONFIG_SCHEMA_VERSION
            )));
        }

        if self.recognition.expected_languages.is_empty() {
            return Err(AppError::config(
                "At least one expected recognition language is required.",
            ));
        }

        let mut normalized_languages = std::collections::HashSet::new();
        for language in &self.recognition.expected_languages {
            let normalized = language.trim().to_ascii_lowercase();
            if normalized.is_empty() {
                return Err(AppError::config(
                    "Expected recognition languages cannot contain an empty value.",
                ));
            }
            if !normalized_languages.insert(normalized) {
                return Err(AppError::config(
                    "Expected recognition languages cannot contain duplicates.",
                ));
            }
        }

        if self.osc.host.trim().is_empty() {
            return Err(AppError::config("OSC host cannot be empty."));
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AudioConfig {
    pub(crate) input_device_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecognitionConfig {
    pub(crate) path: RecognitionPath,
    pub(crate) expected_languages: Vec<String>,
}

impl Default for RecognitionConfig {
    fn default() -> Self {
        Self {
            path: RecognitionPath::OpenAiGptTranscribe,
            expected_languages: default_languages(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum RecognitionPath {
    #[default]
    #[serde(rename = "openai/gpt-transcribe")]
    OpenAiGptTranscribe,
    #[serde(rename = "openai/gpt-live-transcribe")]
    OpenAiGptLiveTranscribe,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PublicationMode {
    #[default]
    Completed,
    Live,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PublicationConfig {
    pub(crate) mode: PublicationMode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OscConfig {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) enabled: bool,
}

impl Default for OscConfig {
    fn default() -> Self {
        Self {
            host: default_osc_host(),
            port: default_osc_port(),
            enabled: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UiConfig {
    pub(crate) show_ongoing_preview: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            show_ongoing_preview: true,
        }
    }
}

fn default_languages() -> Vec<String> {
    vec!["en".to_string()]
}

fn default_osc_host() -> String {
    "127.0.0.1".to_string()
}

fn default_osc_port() -> u16 {
    9000
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
