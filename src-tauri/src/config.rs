//! Non-secret app configuration shared by Tauri commands and the runtime.
//!
//! This module intentionally stores only ordinary settings and non-sensitive
//! metadata. Provider API keys must come from the environment or the system
//! credential store, never this config file.

use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};

pub(crate) const APP_CONFIG_SCHEMA_VERSION: u32 = 3;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AppConfig {
    pub(crate) schema_version: u32,
    pub(crate) audio: AudioConfig,
    pub(crate) stt: SttConfig,
    pub(crate) osc: OscConfig,
    pub(crate) publication: PublicationConfig,
    pub(crate) ui: UiConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: APP_CONFIG_SCHEMA_VERSION,
            audio: AudioConfig::default(),
            stt: SttConfig::default(),
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

        if self.stt.languages.is_empty() {
            return Err(AppError::config(
                "At least one expected STT language is required.",
            ));
        }

        let mut normalized_languages = std::collections::HashSet::new();
        for language in &self.stt.languages {
            let normalized = language.trim().to_ascii_lowercase();
            if normalized.is_empty() {
                return Err(AppError::config(
                    "Expected STT languages cannot contain an empty value.",
                ));
            }
            if !normalized_languages.insert(normalized) {
                return Err(AppError::config(
                    "Expected STT languages cannot contain duplicates.",
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
pub(crate) struct SttConfig {
    pub(crate) provider: SttProvider,
    pub(crate) languages: Vec<String>,
    pub(crate) model: OpenAiTranscriptionModel,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            provider: SttProvider::OpenAi,
            languages: default_languages(),
            model: OpenAiTranscriptionModel::GptTranscribe,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SttProvider {
    #[default]
    #[serde(rename = "openai")]
    OpenAi,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum OpenAiTranscriptionModel {
    #[default]
    #[serde(rename = "gpt-transcribe")]
    GptTranscribe,
    #[serde(rename = "gpt-live-transcribe")]
    GptLiveTranscribe,
}

impl OpenAiTranscriptionModel {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::GptTranscribe => "gpt-transcribe",
            Self::GptLiveTranscribe => "gpt-live-transcribe",
        }
    }
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

impl SttProvider {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
        }
    }
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
    pub(crate) show_partial: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self { show_partial: true }
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
