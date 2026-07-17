use super::*;
use crate::caption_session::CaptionLane;
use crate::config::{PublicationMode, SttConfig, SttProvider};

fn stt(provider: SttProvider, model: &str) -> SttConfig {
    SttConfig {
        provider,
        language: "en".to_string(),
        model: model.to_string(),
    }
}

#[test]
fn bounded_openai_profile_describes_the_complete_adapter_path() {
    let profile = recognition_capabilities(&stt(SttProvider::OpenAi, "any-bounded-model"));

    assert_eq!(profile.path, RecognitionPath::OpenAiBounded);
    assert_eq!(
        profile.input_shape,
        RecognitionInputShape::CompletedAudioUnits
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
fn mock_profiles_cover_every_phase_three_update_shape() {
    let bounded = recognition_capabilities(&stt(SttProvider::Mock, MOCK_BOUNDED_MODEL));
    let ongoing_completed =
        recognition_capabilities(&stt(SttProvider::Mock, MOCK_ONGOING_COMPLETED_MODEL));
    let ongoing_only = recognition_capabilities(&stt(SttProvider::Mock, MOCK_ONGOING_ONLY_MODEL));
    let legacy_mock = recognition_capabilities(&stt(SttProvider::Mock, "saved-before-phase-3"));

    assert_eq!(bounded.path, RecognitionPath::MockBounded);
    assert_eq!(bounded.lanes[0].updates, LaneUpdateBehavior::CompletedOnly);
    assert_eq!(
        ongoing_completed.path,
        RecognitionPath::MockOngoingCompleted
    );
    assert_eq!(
        ongoing_completed.lanes[0].updates,
        LaneUpdateBehavior::OngoingAndCompleted
    );
    assert_eq!(
        ongoing_completed.lanes[0].revisions,
        RevisionBehavior::RevisableFullSnapshot
    );
    assert_eq!(ongoing_only.path, RecognitionPath::MockOngoingOnly);
    assert_eq!(ongoing_only.unit_behavior, CaptionUnitBehavior::Unitless);
    assert_eq!(
        ongoing_only.lanes[0].updates,
        LaneUpdateBehavior::OngoingOnly
    );
    assert_eq!(legacy_mock, ongoing_completed);
}

#[test]
fn completed_only_path_keeps_requested_live_mode_in_an_incompatible_plan() {
    let profile = recognition_capabilities(&stt(SttProvider::OpenAi, "bounded"));
    let plan = plan_publication(&profile, PublicationMode::Live, &[CaptionLane::Source]);

    assert_eq!(
        plan,
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
fn ongoing_plus_completed_supports_both_modes() {
    let profile = recognition_capabilities(&stt(SttProvider::Mock, MOCK_ONGOING_COMPLETED_MODEL));

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
fn ongoing_only_supports_live_without_fabricating_completed_support() {
    let profile = recognition_capabilities(&stt(SttProvider::Mock, MOCK_ONGOING_ONLY_MODEL));

    assert_eq!(
        plan_publication(&profile, PublicationMode::Completed, &[CaptionLane::Source],),
        PublicationPlan::Incompatible {
            requested_mode: PublicationMode::Completed,
            selected_lanes: vec![CaptionLane::Source],
            reason: PublicationIncompatibility::ModeUnsupported {
                lanes: vec![CaptionLane::Source],
            },
            supported_modes: vec![PublicationMode::Live],
        }
    );
    assert!(matches!(
        plan_publication(&profile, PublicationMode::Live, &[CaptionLane::Source]),
        PublicationPlan::Ready {
            policy: ResolvedPublicationPolicy::LiveUnitless {
                first_non_empty_delay_ms: LIVE_OBSERVATION_MILLIS,
            },
            ..
        }
    ));
}

#[test]
fn missing_selected_lane_is_incompatible_in_every_mode() {
    let profile = recognition_capabilities(&stt(SttProvider::OpenAi, "bounded"));

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
    let profile = recognition_capabilities(&stt(SttProvider::OpenAi, "bounded"));

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
fn runtime_plan_keeps_openai_live_as_the_requested_incompatible_mode() {
    let mut config = crate::config::AppConfig::default();
    config.publication.mode = PublicationMode::Live;

    let plan = plan_runtime(&config);

    assert_eq!(plan.recognition.path, RecognitionPath::OpenAiBounded);
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
