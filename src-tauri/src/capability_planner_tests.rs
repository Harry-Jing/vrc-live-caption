use super::*;
use crate::caption_session::CaptionLane;
use crate::config::{OpenAiTranscriptionModel, PublicationMode, SttConfig, SttProvider};

fn stt(model: OpenAiTranscriptionModel) -> SttConfig {
    SttConfig {
        provider: SttProvider::OpenAi,
        languages: vec!["zh".to_string(), "en".to_string()],
        model,
    }
}

#[test]
fn gpt_transcribe_profile_is_completed_only_after_application_commit() {
    let profile = recognition_capabilities(&stt(OpenAiTranscriptionModel::GptTranscribe));

    assert_eq!(profile.path, RecognitionPath::OpenAiGptTranscribe);
    assert_eq!(
        profile.input_shape,
        RecognitionInputShape::ContinuousAudioFrames
    );
    assert_eq!(profile.boundary_owner, BoundaryOwner::Application);
    assert_eq!(profile.unit_behavior, CaptionUnitBehavior::UnitBased);
    assert_eq!(
        profile.lanes,
        vec![LaneCapabilities {
            lane: CaptionLane::Source,
            updates: LaneUpdateBehavior::CompletedOnly,
            revisions: RevisionBehavior::AppendOnly,
        }]
    );
}

#[test]
fn gpt_live_transcribe_profile_exposes_ongoing_and_completed_snapshots() {
    let profile = recognition_capabilities(&stt(OpenAiTranscriptionModel::GptLiveTranscribe));

    assert_eq!(profile.path, RecognitionPath::OpenAiGptLiveTranscribe);
    assert_eq!(
        profile.input_shape,
        RecognitionInputShape::ContinuousAudioFrames
    );
    assert_eq!(profile.boundary_owner, BoundaryOwner::Application);
    assert_eq!(profile.unit_behavior, CaptionUnitBehavior::UnitBased);
    assert_eq!(
        profile.lanes,
        vec![LaneCapabilities {
            lane: CaptionLane::Source,
            updates: LaneUpdateBehavior::OngoingAndCompleted,
            revisions: RevisionBehavior::RevisableFullSnapshot,
        }]
    );
}

#[test]
fn gpt_transcribe_completed_is_ready_and_live_is_explicitly_incompatible() {
    let profile = recognition_capabilities(&stt(OpenAiTranscriptionModel::GptTranscribe));

    assert_eq!(
        plan_publication(&profile, PublicationMode::Completed, &[CaptionLane::Source]),
        PublicationPlan::Ready {
            mode: PublicationMode::Completed,
            policy: ResolvedPublicationPolicy::Completed,
            selected_lanes: vec![CaptionLane::Source],
        }
    );
    assert_eq!(
        plan_publication(&profile, PublicationMode::Live, &[CaptionLane::Source]),
        PublicationPlan::Incompatible {
            requested_mode: PublicationMode::Live,
            selected_lanes: vec![CaptionLane::Source],
            reason: PublicationIncompatibility::ModeUnsupported {
                lanes: vec![CaptionLane::Source],
            },
            supported_modes: vec![PublicationMode::Completed],
        }
    );
}

#[test]
fn gpt_live_transcribe_keeps_both_publication_modes_ready() {
    let profile = recognition_capabilities(&stt(OpenAiTranscriptionModel::GptLiveTranscribe));

    for mode in [PublicationMode::Completed, PublicationMode::Live] {
        assert_eq!(
            plan_publication(&profile, mode, &[CaptionLane::Source]),
            PublicationPlan::Ready {
                mode,
                policy: match mode {
                    PublicationMode::Completed => ResolvedPublicationPolicy::Completed,
                    PublicationMode::Live => ResolvedPublicationPolicy::LiveUnit {
                        observation_window_ms: LIVE_OBSERVATION_MILLIS,
                    },
                },
                selected_lanes: vec![CaptionLane::Source],
            }
        );
    }
}

#[test]
fn missing_selected_lane_is_incompatible_in_every_mode() {
    let profile = recognition_capabilities(&stt(OpenAiTranscriptionModel::GptTranscribe));

    for mode in [PublicationMode::Completed, PublicationMode::Live] {
        assert_eq!(
            plan_publication(&profile, mode, &[CaptionLane::Translation]),
            PublicationPlan::Incompatible {
                requested_mode: mode,
                selected_lanes: vec![CaptionLane::Translation],
                reason: PublicationIncompatibility::LaneUnavailable {
                    lanes: vec![CaptionLane::Translation],
                },
                supported_modes: Vec::new(),
            }
        );
    }
}

#[test]
fn planner_requires_at_least_one_selected_lane() {
    let profile = recognition_capabilities(&stt(OpenAiTranscriptionModel::GptTranscribe));

    assert_eq!(
        plan_publication(&profile, PublicationMode::Completed, &[]),
        PublicationPlan::Incompatible {
            requested_mode: PublicationMode::Completed,
            selected_lanes: Vec::new(),
            reason: PublicationIncompatibility::NoLanesSelected,
            supported_modes: Vec::new(),
        }
    );
}

#[test]
fn runtime_plan_never_rewrites_an_incompatible_model_or_mode() {
    let mut config = crate::config::AppConfig::default();
    config.publication.mode = PublicationMode::Live;

    let plan = plan_runtime(&config);

    assert_eq!(config.stt.model, OpenAiTranscriptionModel::GptTranscribe);
    assert_eq!(plan.recognition.path, RecognitionPath::OpenAiGptTranscribe);
    assert!(matches!(
        plan.publication,
        PublicationPlan::Incompatible {
            requested_mode: PublicationMode::Live,
            ..
        }
    ));
    assert_eq!(
        plan.publication.incompatibility_code(),
        Some("publication.mode_unsupported")
    );
}
