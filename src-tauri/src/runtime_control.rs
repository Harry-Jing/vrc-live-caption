//! Authoritative saved-settings and effective-runtime-session contract.

use crate::capability_planner::RuntimePlanSnapshot;
use crate::config::{AppConfig, AudioConfig, OscConfig, PublicationConfig, SttConfig, SttProvider};
use crate::events::RuntimeStatusEvent;
use crate::secrets::{ProviderSecretStatus, ProviderSecretStorage};
use serde::Serialize;

pub(crate) const RUNTIME_CONTROL_CONTRACT_VERSION: u32 = 3;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeControlSnapshot {
    pub(crate) contract_version: u32,
    pub(crate) revision: u64,
    pub(crate) runtime: RuntimeStatusEvent,
    pub(crate) desired: RuntimeDesiredSnapshot,
    pub(crate) session: Option<RuntimeSessionSnapshot>,
    pub(crate) pending_changes: Vec<PendingSessionChange>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeDesiredSnapshot {
    pub(crate) revision: u64,
    pub(crate) config: AppConfig,
    pub(crate) runtime_plan: RuntimePlanSnapshot,
    pub(crate) provider_secrets: Vec<ProviderSecretStatus>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeSessionSnapshot {
    pub(crate) generation: u64,
    pub(crate) phase: RuntimeSessionPhase,
    pub(crate) started_from_config_revision: u64,
    pub(crate) selected: RuntimeSelectedConfig,
    pub(crate) runtime_plan: RuntimePlanSnapshot,
    pub(crate) credential: Option<RuntimeCredentialSnapshot>,
    pub(crate) chatbox: RuntimeChatboxSnapshot,
    pub(crate) uploads_microphone_audio: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RuntimeSessionPhase {
    Starting,
    Running,
    Reconnecting,
    Stopping,
    Error,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeCredentialSnapshot {
    pub(crate) provider: SttProvider,
    pub(crate) storage: ProviderSecretStorage,
    pub(crate) display_suffix: Option<String>,
    pub(crate) revision: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub(crate) enum RuntimeChatboxSnapshot {
    Disabled {
        host: String,
        port: u16,
    },
    Ready {
        host: String,
        port: u16,
    },
    Unavailable {
        host: String,
        port: u16,
        #[serde(rename = "reasonCode")]
        reason_code: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeSelectedConfig {
    pub(crate) audio: AudioConfig,
    pub(crate) stt: SttConfig,
    pub(crate) osc: OscConfig,
    pub(crate) publication: PublicationConfig,
}

impl From<&AppConfig> for RuntimeSelectedConfig {
    fn from(config: &AppConfig) -> Self {
        Self {
            audio: config.audio.clone(),
            stt: config.stt.clone(),
            osc: config.osc.clone(),
            publication: config.publication.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PendingSessionChange {
    Microphone,
    Recognition,
    Credential,
    ChatboxOutput,
    Publication,
}

pub(crate) fn pending_session_changes(
    desired: &AppConfig,
    selected: &RuntimeSelectedConfig,
    desired_credential_revision: u64,
    session_credential_revision: u64,
) -> Vec<PendingSessionChange> {
    let mut changes = Vec::new();

    if desired.audio != selected.audio {
        changes.push(PendingSessionChange::Microphone);
    }
    if desired.stt != selected.stt {
        changes.push(PendingSessionChange::Recognition);
    }
    if desired_credential_revision != session_credential_revision {
        changes.push(PendingSessionChange::Credential);
    }
    if desired.osc != selected.osc {
        changes.push(PendingSessionChange::ChatboxOutput);
    }
    if desired.publication != selected.publication {
        changes.push(PendingSessionChange::Publication);
    }

    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    #[test]
    fn ui_only_desired_change_does_not_require_session_restart() {
        let selected = RuntimeSelectedConfig::from(&AppConfig::default());
        let mut desired = AppConfig::default();
        desired.ui.show_partial = false;

        assert!(pending_session_changes(&desired, &selected, 0, 0).is_empty());
    }

    #[test]
    fn publication_change_requires_a_new_session_without_becoming_an_osc_change() {
        let selected = RuntimeSelectedConfig::from(&AppConfig::default());
        let mut desired = AppConfig::default();
        desired.publication.mode = crate::config::PublicationMode::Live;

        assert_eq!(
            pending_session_changes(&desired, &selected, 0, 0),
            vec![PendingSessionChange::Publication]
        );
    }

    #[test]
    fn openai_credential_change_requires_a_new_session() {
        let active = AppConfig::default();
        let selected = RuntimeSelectedConfig::from(&active);

        assert_eq!(
            pending_session_changes(&active, &selected, 2, 1),
            vec![PendingSessionChange::Credential]
        );
    }

    #[test]
    fn chatbox_snapshot_uses_the_shared_host_and_port_wire_names() {
        let value = serde_json::to_value(RuntimeChatboxSnapshot::Unavailable {
            host: "127.0.0.1".to_string(),
            port: 9000,
            reason_code: "osc.bind_failed".to_string(),
        })
        .unwrap_or_else(|error| serde_json::json!({ "serializationError": error.to_string() }));

        assert_eq!(value["state"], "unavailable");
        assert_eq!(value["host"], "127.0.0.1");
        assert_eq!(value["port"], 9000);
        assert_eq!(value["reasonCode"], "osc.bind_failed");
        assert!(value.get("requestedHost").is_none());
        assert!(value.get("requestedPort").is_none());
    }
}

#[cfg(test)]
#[path = "runtime_control_contract_tests.rs"]
mod contract_tests;
