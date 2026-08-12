//! Non-secret app configuration shared by Tauri commands and the runtime.
//!
//! This module intentionally stores only ordinary settings and non-sensitive
//! metadata. Service-credential secrets must come from the environment or the
//! system credential store, never this config file.

use crate::error::{AppError, AppResult};
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::fmt;
use url::Url;

pub(crate) const APP_CONFIG_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AppConfig {
    pub(crate) schema_version: u32,
    pub(crate) audio: AudioConfig,
    pub(crate) recognition: RecognitionConfig,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) translation: Option<TranslationConfig>,
    pub(crate) osc: OscConfig,
    pub(crate) publication: PublicationConfig,
    pub(crate) ui: UiConfig,
}

fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: APP_CONFIG_SCHEMA_VERSION,
            audio: AudioConfig::default(),
            recognition: RecognitionConfig::default(),
            translation: None,
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

        if matches!(
            self.publication.content,
            ContentSelection::TranslationOnly | ContentSelection::Bilingual
        ) && self.translation.is_none()
        {
            return Err(AppError::config(
                "Translation content requires a translation selection.",
            ));
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TranslationConfig {
    pub(crate) path: TranslationPath,
    pub(crate) target: TranslationTarget,
    pub(crate) endpoint: TranslationEndpoint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum TranslationPath {
    #[serde(rename = "openai/responses-completed-text")]
    OpenAiResponsesCompletedText,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum TranslationTarget {
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-Hans")]
    SimplifiedChinese,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum TranslationEndpoint {
    Official,
    Custom { api_base_url: ApiBaseUrl },
}

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum TranslationEndpointWire {
    Official {},
    Custom { api_base_url: ApiBaseUrl },
}

impl<'de> Deserialize<'de> for TranslationEndpoint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match TranslationEndpointWire::deserialize(deserializer)? {
            TranslationEndpointWire::Official {} => Ok(Self::Official),
            TranslationEndpointWire::Custom { api_base_url } => Ok(Self::Custom { api_base_url }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ApiBaseUrl(Url);

impl ApiBaseUrl {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        // `url::Url` normalizes an empty userinfo marker away, so inspect the
        // raw authority first to reject every syntactic userinfo form.
        let authority = value
            .split_once("://")
            .map(|(_, remainder)| remainder.split(['/', '?', '#']).next().unwrap_or_default())
            .unwrap_or_default();
        if authority.contains('@') {
            return Err("API base URL cannot contain user information.".to_string());
        }
        let url = Url::parse(value).map_err(|_| "API base URL must be a valid URL.".to_string())?;
        if url.scheme() != "https" {
            return Err("API base URL must use HTTPS.".to_string());
        }
        if !url.has_host() {
            return Err("API base URL must include a host.".to_string());
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("API base URL cannot contain user information.".to_string());
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err("API base URL cannot contain a query or fragment.".to_string());
        }
        if let Some(segment) = url
            .path_segments()
            .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
        {
            if !has_valid_percent_encoding(segment) {
                return Err("API base URL must contain valid percent encoding.".to_string());
            }
            let decoded = percent_decode_str(segment)
                .decode_utf8()
                .map_err(|_| "API base URL must contain valid percent encoding.".to_string())?;
            if decoded.eq_ignore_ascii_case("responses") {
                return Err("API base URL cannot include the Responses endpoint.".to_string());
            }
        }

        Ok(Self(url))
    }
}

fn has_valid_percent_encoding(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return false;
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    true
}

impl Serialize for ApiBaseUrl {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.0.as_str())
    }
}

impl<'de> Deserialize<'de> for ApiBaseUrl {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ApiBaseUrlVisitor;

        impl de::Visitor<'_> for ApiBaseUrlVisitor {
            type Value = ApiBaseUrl;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a verified HTTPS API base URL")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                ApiBaseUrl::parse(value).map_err(E::custom)
            }
        }

        deserializer.deserialize_str(ApiBaseUrlVisitor)
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
    pub(crate) content: ContentSelection,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ContentSelection {
    #[default]
    SourceOnly,
    TranslationOnly,
    Bilingual,
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
