//! Non-secret app configuration shared by Tauri commands and the runtime.
//!
//! This module intentionally stores only ordinary settings such as device ids,
//! STT model names, OSC target, and UI preferences. Provider API keys must come
//! from the environment or a future secure secret store, not this config file.
//! Serde defaults keep older config files loadable as Phase 1 fields evolve.

use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppConfig {
    #[serde(default)]
    pub(crate) audio: AudioConfig,
    #[serde(default)]
    pub(crate) stt: SttConfig,
    #[serde(default)]
    pub(crate) osc: OscConfig,
    #[serde(default)]
    pub(crate) ui: UiConfig,
}

impl AppConfig {
    pub(crate) fn validate(&self) -> AppResult<()> {
        if self.stt.language.trim().is_empty() {
            return Err(AppError::config("STT language cannot be empty."));
        }

        if self.stt.model.trim().is_empty() {
            return Err(AppError::config("STT model cannot be empty."));
        }

        if self.osc.host.trim().is_empty() {
            return Err(AppError::config("OSC host cannot be empty."));
        }

        if self.osc.min_interval_ms < 500 {
            return Err(AppError::config(
                "OSC minimum interval must be at least 500 ms.",
            ));
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AudioConfig {
    pub(crate) input_device_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SttConfig {
    #[serde(default)]
    pub(crate) provider: SttProvider,
    #[serde(default = "default_language")]
    pub(crate) language: String,
    #[serde(default = "default_stt_model")]
    pub(crate) model: String,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            provider: SttProvider::OpenAi,
            language: default_language(),
            model: default_stt_model(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SttProvider {
    Mock,
    #[default]
    #[serde(rename = "openai", alias = "cloud")]
    OpenAi,
}

impl SttProvider {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::OpenAi => "openai",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OscConfig {
    #[serde(default = "default_osc_host")]
    pub(crate) host: String,
    #[serde(default = "default_osc_port")]
    pub(crate) port: u16,
    #[serde(default = "default_true")]
    pub(crate) enabled: bool,
    #[serde(default = "default_osc_min_interval_ms")]
    pub(crate) min_interval_ms: u64,
}

impl Default for OscConfig {
    fn default() -> Self {
        Self {
            host: default_osc_host(),
            port: default_osc_port(),
            enabled: true,
            min_interval_ms: default_osc_min_interval_ms(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UiConfig {
    #[serde(default = "default_true")]
    pub(crate) show_partial: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self { show_partial: true }
    }
}

fn default_language() -> String {
    "en".to_string()
}

fn default_stt_model() -> String {
    "gpt-4o-mini-transcribe".to_string()
}

fn default_osc_host() -> String {
    "127.0.0.1".to_string()
}

fn default_osc_port() -> u16 {
    9000
}

fn default_osc_min_interval_ms() -> u64 {
    1200
}

fn default_true() -> bool {
    true
}
