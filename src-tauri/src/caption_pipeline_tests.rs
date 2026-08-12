use super::*;
use crate::caption::CaptionLane;
use crate::config::{
    ContentSelection, PublicationMode, RecognitionConfig, RecognitionPath, TranslationConfig,
    TranslationEndpoint, TranslationPath, TranslationTarget,
};

fn recognition(path: RecognitionPath) -> RecognitionConfig {
    RecognitionConfig {
        path,
        expected_languages: vec!["zh".to_string(), "en".to_string()],
    }
}

#[test]
fn gpt_transcribe_profile_is_completed_only_after_application_commit() {
    let profile = recognition_capabilities(&recognition(RecognitionPath::OpenAiGptTranscribe));

    assert_eq!(profile.path, RecognitionPath::OpenAiGptTranscribe);
    assert_eq!(
        profile.input_shape,
        RecognitionInputShape::ContinuousAudioFrames
    );
    assert_eq!(
        profile.caption_boundary_owner,
        CaptionBoundaryOwner::Application
    );
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
    let profile = recognition_capabilities(&recognition(RecognitionPath::OpenAiGptLiveTranscribe));

    assert_eq!(profile.path, RecognitionPath::OpenAiGptLiveTranscribe);
    assert_eq!(
        profile.input_shape,
        RecognitionInputShape::ContinuousAudioFrames
    );
    assert_eq!(
        profile.caption_boundary_owner,
        CaptionBoundaryOwner::Application
    );
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
    let profile = recognition_capabilities(&recognition(RecognitionPath::OpenAiGptTranscribe));

    assert_eq!(
        plan_publication(
            &profile.lanes,
            PublicationMode::Completed,
            &[CaptionLane::Source],
        ),
        PublicationPlan::Compatible {
            mode: PublicationMode::Completed,
            timing: ResolvedPublicationTiming::Completed,
            selected_lanes: vec![CaptionLane::Source],
        }
    );
    assert_eq!(
        plan_publication(
            &profile.lanes,
            PublicationMode::Live,
            &[CaptionLane::Source]
        ),
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
    let profile = recognition_capabilities(&recognition(RecognitionPath::OpenAiGptLiveTranscribe));

    for mode in [PublicationMode::Completed, PublicationMode::Live] {
        assert_eq!(
            plan_publication(&profile.lanes, mode, &[CaptionLane::Source]),
            PublicationPlan::Compatible {
                mode,
                timing: match mode {
                    PublicationMode::Completed => ResolvedPublicationTiming::Completed,
                    PublicationMode::Live => ResolvedPublicationTiming::LiveUnit {
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
    let profile = recognition_capabilities(&recognition(RecognitionPath::OpenAiGptTranscribe));

    for mode in [PublicationMode::Completed, PublicationMode::Live] {
        assert_eq!(
            plan_publication(&profile.lanes, mode, &[CaptionLane::Translation]),
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
fn caption_pipeline_requires_at_least_one_selected_lane() {
    let profile = recognition_capabilities(&recognition(RecognitionPath::OpenAiGptTranscribe));

    assert_eq!(
        plan_publication(&profile.lanes, PublicationMode::Completed, &[]),
        PublicationPlan::Incompatible {
            requested_mode: PublicationMode::Completed,
            selected_lanes: Vec::new(),
            reason: PublicationIncompatibility::NoLanesSelected,
            supported_modes: Vec::new(),
        }
    );
}

#[test]
fn translation_only_completed_activates_the_completed_translation_profile() {
    let mut config = crate::config::AppConfig {
        translation: Some(TranslationConfig {
            path: TranslationPath::OpenAiResponsesCompletedText,
            target: TranslationTarget::SimplifiedChinese,
            endpoint: TranslationEndpoint::Official,
        }),
        ..Default::default()
    };
    config.publication.content = ContentSelection::TranslationOnly;

    let plan = plan_caption_pipeline(&config);

    assert_eq!(
        plan.translation,
        Some(TranslationCapabilityProfile {
            path: TranslationPath::OpenAiResponsesCompletedText,
            input_shape: TranslationInputShape::CompletedSourceSnapshots,
            lanes: vec![LaneCapabilities {
                lane: CaptionLane::Translation,
                updates: LaneUpdateBehavior::CompletedOnly,
                revisions: RevisionBehavior::AppendOnly,
            }],
        })
    );
    assert_eq!(
        plan.publication,
        PublicationPlan::Compatible {
            mode: PublicationMode::Completed,
            timing: ResolvedPublicationTiming::Completed,
            selected_lanes: vec![CaptionLane::Translation],
        }
    );
}

#[test]
fn bilingual_live_reports_translation_as_the_unsupported_lane() {
    let mut config = crate::config::AppConfig::default();
    config.recognition.path = RecognitionPath::OpenAiGptLiveTranscribe;
    config.translation = Some(TranslationConfig {
        path: TranslationPath::OpenAiResponsesCompletedText,
        target: TranslationTarget::English,
        endpoint: TranslationEndpoint::Official,
    });
    config.publication.mode = PublicationMode::Live;
    config.publication.content = ContentSelection::Bilingual;

    let plan = plan_caption_pipeline(&config);

    assert_eq!(
        plan.publication,
        PublicationPlan::Incompatible {
            requested_mode: PublicationMode::Live,
            selected_lanes: vec![CaptionLane::Source, CaptionLane::Translation],
            reason: PublicationIncompatibility::ModeUnsupported {
                lanes: vec![CaptionLane::Translation],
            },
            supported_modes: vec![PublicationMode::Completed],
        }
    );
    assert_eq!(config.publication.mode, PublicationMode::Live);
    assert_eq!(config.publication.content, ContentSelection::Bilingual);
}

#[test]
fn remaining_translation_content_mode_cells_are_planned_explicitly() {
    let cases = [
        (
            ContentSelection::TranslationOnly,
            PublicationMode::Live,
            PublicationPlan::Incompatible {
                requested_mode: PublicationMode::Live,
                selected_lanes: vec![CaptionLane::Translation],
                reason: PublicationIncompatibility::ModeUnsupported {
                    lanes: vec![CaptionLane::Translation],
                },
                supported_modes: vec![PublicationMode::Completed],
            },
        ),
        (
            ContentSelection::Bilingual,
            PublicationMode::Completed,
            PublicationPlan::Compatible {
                mode: PublicationMode::Completed,
                timing: ResolvedPublicationTiming::Completed,
                selected_lanes: vec![CaptionLane::Source, CaptionLane::Translation],
            },
        ),
    ];

    for (content, mode, expected) in cases {
        let mut config = crate::config::AppConfig::default();
        config.recognition.path = RecognitionPath::OpenAiGptLiveTranscribe;
        config.translation = Some(TranslationConfig {
            path: TranslationPath::OpenAiResponsesCompletedText,
            target: TranslationTarget::English,
            endpoint: TranslationEndpoint::Official,
        });
        config.publication.mode = mode;
        config.publication.content = content;

        let plan = plan_caption_pipeline(&config);

        assert!(plan.translation.is_some());
        assert_eq!(plan.publication, expected);
        assert_eq!(config.publication.mode, mode);
        assert_eq!(config.publication.content, content);
    }
}

#[test]
fn source_only_keeps_saved_translation_dormant() {
    let mut config = crate::config::AppConfig::default();
    config.recognition.path = RecognitionPath::OpenAiGptLiveTranscribe;
    config.translation = Some(TranslationConfig {
        path: TranslationPath::OpenAiResponsesCompletedText,
        target: TranslationTarget::SimplifiedChinese,
        endpoint: TranslationEndpoint::Official,
    });
    config.publication.mode = PublicationMode::Live;

    let plan = plan_caption_pipeline(&config);

    assert_eq!(plan.translation, None);
    assert_eq!(
        plan.publication,
        PublicationPlan::Compatible {
            mode: PublicationMode::Live,
            timing: ResolvedPublicationTiming::LiveUnit {
                observation_window_ms: LIVE_OBSERVATION_MILLIS,
            },
            selected_lanes: vec![CaptionLane::Source],
        }
    );
}

#[test]
fn active_translation_fails_start_until_the_translation_module_exists()
-> crate::error::AppResult<()> {
    let mut config = crate::config::AppConfig {
        translation: Some(TranslationConfig {
            path: TranslationPath::OpenAiResponsesCompletedText,
            target: TranslationTarget::SimplifiedChinese,
            endpoint: TranslationEndpoint::Official,
        }),
        ..Default::default()
    };
    config.publication.content = ContentSelection::TranslationOnly;
    let plan = plan_caption_pipeline(&config);

    let error = resolve_caption_pipeline_start_timing(&plan)
        .err()
        .ok_or_else(|| {
            crate::error::AppError::state("Translation unexpectedly became startable.")
        })?;

    assert_eq!(error.code(), "config.invalid");
    assert!(error.to_string().contains("translation.module_unavailable"));
    Ok(())
}

#[test]
fn incompatible_plan_has_one_startability_error() -> crate::error::AppResult<()> {
    let mut config = crate::config::AppConfig::default();
    config.publication.mode = PublicationMode::Live;

    let error = resolve_caption_pipeline_start_timing(&plan_caption_pipeline(&config))
        .err()
        .ok_or_else(|| {
            crate::error::AppError::state(
                "An incompatible Caption Pipeline Plan unexpectedly became startable.",
            )
        })?;

    assert_eq!(error.code(), "config.invalid");
    assert!(error.to_string().contains("publication.mode_unsupported"));
    Ok(())
}

#[test]
fn caption_pipeline_plan_never_rewrites_an_incompatible_path_or_mode() {
    let mut config = crate::config::AppConfig::default();
    config.publication.mode = PublicationMode::Live;

    let plan = plan_caption_pipeline(&config);

    assert_eq!(
        config.recognition.path,
        RecognitionPath::OpenAiGptTranscribe
    );
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
