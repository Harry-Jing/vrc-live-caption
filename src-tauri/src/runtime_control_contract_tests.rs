use super::*;
use crate::capability_planner::{
    PublicationIncompatibility, PublicationPlan, ResolvedPublicationPolicy, plan_runtime,
};
use crate::caption_session::CaptionLane;
use crate::config::{AppConfig, OpenAiTranscriptionModel, PublicationMode, SttProvider};
use crate::events::RuntimeStatus;
use crate::secrets::{ProviderSecretStatus, ProviderSecretStorage};

#[test]
fn shared_v3_fixture_matches_the_rust_serializer() -> Result<(), serde_json::Error> {
    let mut config = AppConfig::default();
    config.stt.languages = vec!["zh".to_string(), "en".to_string()];
    config.stt.model = OpenAiTranscriptionModel::GptLiveTranscribe;
    config.publication.mode = PublicationMode::Live;
    let runtime_plan = plan_runtime(&config);
    let snapshot = RuntimeControlSnapshot {
        contract_version: RUNTIME_CONTROL_CONTRACT_VERSION,
        revision: 9,
        runtime: RuntimeStatusEvent {
            status: RuntimeStatus::Running,
            message: Some("Runtime is running".to_string()),
            timestamp_ms: 900,
        },
        desired: RuntimeDesiredSnapshot {
            revision: 4,
            config: config.clone(),
            runtime_plan: runtime_plan.clone(),
            provider_secrets: vec![ProviderSecretStatus {
                provider: "openai".to_string(),
                configured: true,
                storage: Some(ProviderSecretStorage::SystemCredentialStore),
                display_suffix: Some("abcd".to_string()),
                error: None,
            }],
        },
        session: Some(RuntimeSessionSnapshot {
            generation: 3,
            phase: RuntimeSessionPhase::Running,
            started_from_config_revision: 4,
            selected: RuntimeSelectedConfig::from(&config),
            runtime_plan,
            credential: Some(RuntimeCredentialSnapshot {
                provider: SttProvider::OpenAi,
                storage: ProviderSecretStorage::SystemCredentialStore,
                display_suffix: Some("abcd".to_string()),
                revision: 2,
            }),
            chatbox: RuntimeChatboxSnapshot::Ready {
                host: "127.0.0.1".to_string(),
                port: 9000,
            },
            uploads_microphone_audio: true,
        }),
        pending_changes: Vec::new(),
    };
    let expected = serde_json::from_str::<serde_json::Value>(include_str!(
        "../../contracts/runtime-control-snapshot-v3.json"
    ))?;

    assert_eq!(serde_json::to_value(snapshot)?, expected);
    Ok(())
}

#[test]
fn every_struct_variant_field_uses_camel_case() -> Result<(), serde_json::Error> {
    assert_eq!(
        serde_json::to_value(ResolvedPublicationPolicy::LiveUnit {
            observation_window_ms: 1_000,
        })?,
        serde_json::json!({
            "policy": "liveUnit",
            "observationWindowMs": 1_000,
        })
    );
    assert_eq!(
        serde_json::to_value(PublicationPlan::Incompatible {
            requested_mode: PublicationMode::Live,
            selected_lanes: vec![CaptionLane::Source],
            reason: PublicationIncompatibility::ModeUnsupported {
                lanes: vec![CaptionLane::Source],
            },
            supported_modes: vec![PublicationMode::Completed],
        })?,
        serde_json::json!({
            "state": "incompatible",
            "requestedMode": "live",
            "selectedLanes": ["source"],
            "reason": {
                "reason": "modeUnsupported",
                "lanes": ["source"],
            },
            "supportedModes": ["completed"],
        })
    );
    Ok(())
}
