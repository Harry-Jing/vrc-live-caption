use super::*;
use crate::caption::{CaptionLane, CaptionState};
use crate::caption_pipeline::{
    CaptionBoundaryOwner, CaptionUnitBehavior, LaneUpdateBehavior, PublicationIncompatibility,
    PublicationPlan, RecognitionInputShape, ResolvedPublicationTiming, RevisionBehavior,
    plan_caption_pipeline,
};
use crate::config::{AppConfig, PublicationMode, RecognitionPath};
use crate::credentials::{CredentialFailure, CredentialId, CredentialStatus, CredentialStorage};
use crate::error::{AppError, AppResult};
use crate::events::{DiagnosticCategory, DiagnosticSeverity};

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
        "../../contracts/wire-vocabulary-v2.json"
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
        "credentialIds": serialized_wire_values(exhaustive_values!(CredentialId;
            CredentialId::OpenAi => CredentialId::OpenAi,
        ))?,
        "credentialStorages": serialized_wire_values(exhaustive_values!(CredentialStorage;
            CredentialStorage::SystemCredentialStore => CredentialStorage::SystemCredentialStore,
            CredentialStorage::Environment => CredentialStorage::Environment,
        ))?,
        "credentialStatusStates": serialized_tag_values(exhaustive_values!(CredentialStatus;
            CredentialStatus::Unconfigured { .. } => CredentialStatus::Unconfigured {
                id: CredentialId::OpenAi,
            },
            CredentialStatus::Configured { .. } => CredentialStatus::Configured {
                id: CredentialId::OpenAi,
                storage: CredentialStorage::SystemCredentialStore,
                display_suffix: None,
            },
            CredentialStatus::Unavailable { .. } => CredentialStatus::Unavailable {
                id: CredentialId::OpenAi,
                failure: CredentialFailure {
                    code: String::new(),
                    message: String::new(),
                },
            },
        ), "state")?,
        "diagnosticCategories": serialized_wire_values(exhaustive_values!(DiagnosticCategory;
            DiagnosticCategory::Config => DiagnosticCategory::Config,
            DiagnosticCategory::Runtime => DiagnosticCategory::Runtime,
            DiagnosticCategory::Audio => DiagnosticCategory::Audio,
            DiagnosticCategory::Recognition => DiagnosticCategory::Recognition,
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
        "captionBoundaryOwners": serialized_wire_values(exhaustive_values!(CaptionBoundaryOwner;
            CaptionBoundaryOwner::Application => CaptionBoundaryOwner::Application,
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
        "resolvedPublicationTimings": serialized_tag_values(exhaustive_values!(ResolvedPublicationTiming;
            ResolvedPublicationTiming::Completed => ResolvedPublicationTiming::Completed,
            ResolvedPublicationTiming::LiveUnit { .. } => ResolvedPublicationTiming::LiveUnit { observation_window_ms: 1 },
        ), "timing")?,
        "publicationPlanStates": serialized_tag_values(exhaustive_values!(PublicationPlan;
            PublicationPlan::Compatible { .. } => PublicationPlan::Compatible {
                mode: PublicationMode::Completed,
                timing: ResolvedPublicationTiming::Completed,
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
        "runtimePendingGenerationChanges": serialized_wire_values(exhaustive_values!(PendingGenerationChange;
            PendingGenerationChange::Microphone => PendingGenerationChange::Microphone,
            PendingGenerationChange::Recognition => PendingGenerationChange::Recognition,
            PendingGenerationChange::Credential => PendingGenerationChange::Credential,
            PendingGenerationChange::ChatboxOutput => PendingGenerationChange::ChatboxOutput,
            PendingGenerationChange::Publication => PendingGenerationChange::Publication,
        ))?,
        "runtimeGenerationPhases": serialized_wire_values(exhaustive_values!(RuntimeGenerationPhase;
            RuntimeGenerationPhase::Starting => RuntimeGenerationPhase::Starting,
            RuntimeGenerationPhase::Running => RuntimeGenerationPhase::Running,
            RuntimeGenerationPhase::Reconnecting => RuntimeGenerationPhase::Reconnecting,
            RuntimeGenerationPhase::Stopping => RuntimeGenerationPhase::Stopping,
            RuntimeGenerationPhase::Error => RuntimeGenerationPhase::Error,
        ))?,
        "chatboxPublicationStates": serialized_tag_values(exhaustive_values!(ChatboxPublicationSnapshot;
            ChatboxPublicationSnapshot::Disabled { .. } => ChatboxPublicationSnapshot::Disabled { host: String::new(), port: 0 },
            ChatboxPublicationSnapshot::Ready { .. } => ChatboxPublicationSnapshot::Ready { host: String::new(), port: 0 },
            ChatboxPublicationSnapshot::Unavailable { .. } => ChatboxPublicationSnapshot::Unavailable {
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
fn shared_v4_fixture_matches_the_rust_serializer() -> Result<(), serde_json::Error> {
    let mut config = AppConfig::default();
    config.recognition.expected_languages = vec!["zh".to_string(), "en".to_string()];
    config.recognition.path = RecognitionPath::OpenAiGptLiveTranscribe;
    config.publication.mode = PublicationMode::Live;
    let caption_pipeline_plan = plan_caption_pipeline(&config);
    let snapshot = RuntimeControlSnapshot {
        contract_version: RUNTIME_CONTROL_CONTRACT_VERSION,
        revision: 9,
        runtime_status: RuntimeStatusEvent {
            status: RuntimeStatus::Running,
            message: Some("Runtime is running".to_string()),
            timestamp_ms: 900,
        },
        desired: RuntimeDesiredSnapshot {
            revision: 4,
            config: config.clone(),
            caption_pipeline_plan: caption_pipeline_plan.clone(),
            credentials: vec![CredentialStatus::Configured {
                id: CredentialId::OpenAi,
                storage: CredentialStorage::SystemCredentialStore,
                display_suffix: Some("abcd".to_string()),
            }],
        },
        generation: Some(RuntimeGenerationSnapshot {
            id: 3,
            phase: RuntimeGenerationPhase::Running,
            started_from_config_revision: 4,
            selection: RuntimeGenerationSelection::from(&config),
            caption_pipeline_plan,
            credential: Some(RuntimeGenerationCredentialSnapshot {
                id: CredentialId::OpenAi,
                storage: CredentialStorage::SystemCredentialStore,
                display_suffix: Some("abcd".to_string()),
                revision: 2,
            }),
            chatbox_publication: ChatboxPublicationSnapshot::Ready {
                host: "127.0.0.1".to_string(),
                port: 9000,
            },
            uploads_microphone_audio: true,
        }),
        pending_generation_changes: Vec::new(),
    };
    let expected = serde_json::from_str::<serde_json::Value>(include_str!(
        "../../contracts/runtime-control-snapshot-v4.json"
    ))?;

    assert_eq!(serde_json::to_value(snapshot)?, expected);
    Ok(())
}

#[test]
fn every_struct_variant_field_uses_camel_case() -> Result<(), serde_json::Error> {
    assert_eq!(
        serde_json::to_value(ResolvedPublicationTiming::LiveUnit {
            observation_window_ms: 1_000,
        })?,
        serde_json::json!({
            "timing": "liveUnit",
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
