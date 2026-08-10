use super::*;
use crate::capability_planner::{
    BoundaryOwner, CaptionUnitBehavior, LaneUpdateBehavior, PublicationIncompatibility,
    PublicationPlan, RecognitionInputShape, RecognitionPath, ResolvedPublicationPolicy,
    RevisionBehavior, plan_runtime,
};
use crate::caption_session::{CaptionLane, CaptionState};
use crate::config::{AppConfig, OpenAiTranscriptionModel, PublicationMode, SttProvider};
use crate::error::{AppError, AppResult};
use crate::events::{DiagnosticCategory, DiagnosticSeverity};
use crate::secrets::{ProviderSecretStatus, ProviderSecretStorage};

macro_rules! exhaustive_values {
    ($type:ty; $($pattern:pat => $value:expr),+ $(,)?) => {{
        fn assert_exhaustive(value: &$type) {
            match value {
                $($pattern => {}),+
            }
        }

        let values = [$($value),+];
        for value in &values {
            assert_exhaustive(value);
        }
        values
    }};
}

fn serialized_wire_values<T: serde::Serialize, const N: usize>(
    values: [T; N],
) -> AppResult<serde_json::Value> {
    serde_json::to_value(values.as_slice())
        .map_err(|error| AppError::config(format!("Failed to serialize wire values: {error}")))
}

fn serialized_tag_values<T: serde::Serialize, const N: usize>(
    values: [T; N],
    tag: &str,
) -> AppResult<serde_json::Value> {
    values
        .into_iter()
        .map(|value| {
            let serialized = serde_json::to_value(value).map_err(|error| {
                AppError::config(format!("Failed to serialize tagged wire value: {error}"))
            })?;
            serialized
                .get(tag)
                .and_then(serde_json::Value::as_str)
                .map(|value| serde_json::Value::String(value.to_string()))
                .ok_or_else(|| {
                    AppError::state(format!(
                        "Serialized wire variant must contain the `{tag}` tag"
                    ))
                })
        })
        .collect::<AppResult<Vec<_>>>()
        .map(serde_json::Value::Array)
}

#[test]
fn closed_rust_wire_values_match_the_shared_vocabulary() -> AppResult<()> {
    let expected = serde_json::from_str::<serde_json::Value>(include_str!(
        "../../contracts/wire-vocabulary-v1.json"
    ))
    .map_err(|error| AppError::config(format!("Failed to parse wire vocabulary: {error}")))?;
    let actual = serde_json::json!({
        "runtimeStatuses": serialized_wire_values(exhaustive_values!(RuntimeStatus;
            RuntimeStatus::Idle => RuntimeStatus::Idle,
            RuntimeStatus::Starting => RuntimeStatus::Starting,
            RuntimeStatus::Running => RuntimeStatus::Running,
            RuntimeStatus::Reconnecting => RuntimeStatus::Reconnecting,
            RuntimeStatus::Stopping => RuntimeStatus::Stopping,
            RuntimeStatus::Stopped => RuntimeStatus::Stopped,
            RuntimeStatus::Error => RuntimeStatus::Error,
        ))?,
        "sttProviders": serialized_wire_values(exhaustive_values!(SttProvider;
            SttProvider::OpenAi => SttProvider::OpenAi,
        ))?,
        "openAiTranscriptionModels": serialized_wire_values(exhaustive_values!(OpenAiTranscriptionModel;
            OpenAiTranscriptionModel::GptTranscribe => OpenAiTranscriptionModel::GptTranscribe,
            OpenAiTranscriptionModel::GptLiveTranscribe => OpenAiTranscriptionModel::GptLiveTranscribe,
        ))?,
        "providerSecretStorages": serialized_wire_values(exhaustive_values!(ProviderSecretStorage;
            ProviderSecretStorage::SystemCredentialStore => ProviderSecretStorage::SystemCredentialStore,
            ProviderSecretStorage::Environment => ProviderSecretStorage::Environment,
        ))?,
        "diagnosticCategories": serialized_wire_values(exhaustive_values!(DiagnosticCategory;
            DiagnosticCategory::Config => DiagnosticCategory::Config,
            DiagnosticCategory::Runtime => DiagnosticCategory::Runtime,
            DiagnosticCategory::Audio => DiagnosticCategory::Audio,
            DiagnosticCategory::Stt => DiagnosticCategory::Stt,
            DiagnosticCategory::Osc => DiagnosticCategory::Osc,
        ))?,
        "diagnosticSeverities": serialized_wire_values(exhaustive_values!(DiagnosticSeverity;
            DiagnosticSeverity::Info => DiagnosticSeverity::Info,
            DiagnosticSeverity::Warning => DiagnosticSeverity::Warning,
            DiagnosticSeverity::Error => DiagnosticSeverity::Error,
        ))?,
        "captionLanes": serialized_wire_values(exhaustive_values!(CaptionLane;
            CaptionLane::Source => CaptionLane::Source,
            CaptionLane::Translation => CaptionLane::Translation,
        ))?,
        "captionStates": serialized_wire_values(exhaustive_values!(CaptionState;
            CaptionState::Ongoing => CaptionState::Ongoing,
            CaptionState::Completed => CaptionState::Completed,
        ))?,
        "publicationModes": serialized_wire_values(exhaustive_values!(PublicationMode;
            PublicationMode::Completed => PublicationMode::Completed,
            PublicationMode::Live => PublicationMode::Live,
        ))?,
        "recognitionPaths": serialized_wire_values(exhaustive_values!(RecognitionPath;
            RecognitionPath::OpenAiGptTranscribe => RecognitionPath::OpenAiGptTranscribe,
            RecognitionPath::OpenAiGptLiveTranscribe => RecognitionPath::OpenAiGptLiveTranscribe,
        ))?,
        "recognitionInputShapes": serialized_wire_values(exhaustive_values!(RecognitionInputShape;
            RecognitionInputShape::ContinuousAudioFrames => RecognitionInputShape::ContinuousAudioFrames,
        ))?,
        "boundaryOwners": serialized_wire_values(exhaustive_values!(BoundaryOwner;
            BoundaryOwner::Application => BoundaryOwner::Application,
        ))?,
        "captionUnitBehaviors": serialized_wire_values(exhaustive_values!(CaptionUnitBehavior;
            CaptionUnitBehavior::UnitBased => CaptionUnitBehavior::UnitBased,
        ))?,
        "laneUpdateBehaviors": serialized_wire_values(exhaustive_values!(LaneUpdateBehavior;
            LaneUpdateBehavior::CompletedOnly => LaneUpdateBehavior::CompletedOnly,
            LaneUpdateBehavior::OngoingAndCompleted => LaneUpdateBehavior::OngoingAndCompleted,
        ))?,
        "revisionBehaviors": serialized_wire_values(exhaustive_values!(RevisionBehavior;
            RevisionBehavior::AppendOnly => RevisionBehavior::AppendOnly,
            RevisionBehavior::RevisableFullSnapshot => RevisionBehavior::RevisableFullSnapshot,
        ))?,
        "resolvedPublicationPolicies": serialized_tag_values(exhaustive_values!(ResolvedPublicationPolicy;
            ResolvedPublicationPolicy::Completed => ResolvedPublicationPolicy::Completed,
            ResolvedPublicationPolicy::LiveUnit { .. } => ResolvedPublicationPolicy::LiveUnit { observation_window_ms: 1 },
        ), "policy")?,
        "publicationPlanStates": serialized_tag_values(exhaustive_values!(PublicationPlan;
            PublicationPlan::Ready { .. } => PublicationPlan::Ready {
                mode: PublicationMode::Completed,
                policy: ResolvedPublicationPolicy::Completed,
                selected_lanes: Vec::new(),
            },
            PublicationPlan::Incompatible { .. } => PublicationPlan::Incompatible {
                requested_mode: PublicationMode::Live,
                selected_lanes: Vec::new(),
                reason: PublicationIncompatibility::NoLanesSelected,
                supported_modes: Vec::new(),
            },
        ), "state")?,
        "publicationIncompatibilityReasons": serialized_tag_values(exhaustive_values!(PublicationIncompatibility;
            PublicationIncompatibility::NoLanesSelected => PublicationIncompatibility::NoLanesSelected,
            PublicationIncompatibility::LaneUnavailable { .. } => PublicationIncompatibility::LaneUnavailable { lanes: Vec::new() },
            PublicationIncompatibility::ModeUnsupported { .. } => PublicationIncompatibility::ModeUnsupported { lanes: Vec::new() },
        ), "reason")?,
        "runtimePendingChanges": serialized_wire_values(exhaustive_values!(PendingSessionChange;
            PendingSessionChange::Microphone => PendingSessionChange::Microphone,
            PendingSessionChange::Recognition => PendingSessionChange::Recognition,
            PendingSessionChange::Credential => PendingSessionChange::Credential,
            PendingSessionChange::ChatboxOutput => PendingSessionChange::ChatboxOutput,
            PendingSessionChange::Publication => PendingSessionChange::Publication,
        ))?,
        "runtimeSessionPhases": serialized_wire_values(exhaustive_values!(RuntimeSessionPhase;
            RuntimeSessionPhase::Starting => RuntimeSessionPhase::Starting,
            RuntimeSessionPhase::Running => RuntimeSessionPhase::Running,
            RuntimeSessionPhase::Reconnecting => RuntimeSessionPhase::Reconnecting,
            RuntimeSessionPhase::Stopping => RuntimeSessionPhase::Stopping,
            RuntimeSessionPhase::Error => RuntimeSessionPhase::Error,
        ))?,
        "runtimeChatboxStates": serialized_tag_values(exhaustive_values!(RuntimeChatboxSnapshot;
            RuntimeChatboxSnapshot::Disabled { .. } => RuntimeChatboxSnapshot::Disabled { host: String::new(), port: 0 },
            RuntimeChatboxSnapshot::Ready { .. } => RuntimeChatboxSnapshot::Ready { host: String::new(), port: 0 },
            RuntimeChatboxSnapshot::Unavailable { .. } => RuntimeChatboxSnapshot::Unavailable {
                host: String::new(),
                port: 0,
                reason_code: String::new(),
            },
        ), "state")?,
    });

    assert_eq!(actual, expected);
    Ok(())
}

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
