use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppConfig {
    pub(crate) audio: AudioConfig,
    pub(crate) stt: SttConfig,
    pub(crate) osc: OscConfig,
    pub(crate) ui: UiConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            audio: AudioConfig {
                input_device_id: None,
            },
            stt: SttConfig {
                provider: SttProvider::Mock,
                language: "en-US".to_string(),
            },
            osc: OscConfig {
                host: "127.0.0.1".to_string(),
                port: 9000,
                enabled: false,
            },
            ui: UiConfig { show_partial: true },
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AudioConfig {
    pub(crate) input_device_id: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SttConfig {
    pub(crate) provider: SttProvider,
    pub(crate) language: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SttProvider {
    Mock,
    Cloud,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OscConfig {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) enabled: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UiConfig {
    pub(crate) show_partial: bool,
}
