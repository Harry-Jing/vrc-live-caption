//! Caption-pipeline capabilities and publication compatibility planning.

use crate::caption::CaptionLane;
use crate::config::{
    AppConfig, ContentSelection, PublicationMode, RecognitionConfig, RecognitionPath,
    TranslationConfig, TranslationPath,
};
use crate::error::{AppError, AppResult};
use serde::Serialize;

pub(crate) const LIVE_OBSERVATION_MILLIS: u64 = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RecognitionInputShape {
    ContinuousAudioFrames,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum TranslationInputShape {
    CompletedSourceSnapshots,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CaptionBoundaryOwner {
    Application,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CaptionUnitBehavior {
    UnitBased,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RevisionBehavior {
    AppendOnly,
    RevisableFullSnapshot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum LaneUpdateBehavior {
    CompletedOnly,
    OngoingAndCompleted,
}

impl LaneUpdateBehavior {
    fn supports(self, mode: PublicationMode) -> bool {
        match mode {
            PublicationMode::Completed => {
                matches!(self, Self::CompletedOnly | Self::OngoingAndCompleted)
            }
            PublicationMode::Live => matches!(self, Self::OngoingAndCompleted),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LaneCapabilities {
    pub(crate) lane: CaptionLane,
    pub(crate) updates: LaneUpdateBehavior,
    pub(crate) revisions: RevisionBehavior,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecognitionCapabilityProfile {
    pub(crate) path: RecognitionPath,
    pub(crate) input_shape: RecognitionInputShape,
    pub(crate) caption_boundary_owner: CaptionBoundaryOwner,
    pub(crate) unit_behavior: CaptionUnitBehavior,
    pub(crate) lanes: Vec<LaneCapabilities>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TranslationCapabilityProfile {
    pub(crate) path: TranslationPath,
    pub(crate) input_shape: TranslationInputShape,
    pub(crate) lanes: Vec<LaneCapabilities>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "camelCase")]
pub(crate) enum PublicationIncompatibility {
    NoLanesSelected,
    LaneUnavailable { lanes: Vec<CaptionLane> },
    ModeUnsupported { lanes: Vec<CaptionLane> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "timing",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum ResolvedPublicationTiming {
    Completed,
    LiveUnit { observation_window_ms: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum PublicationPlan {
    Compatible {
        mode: PublicationMode,
        timing: ResolvedPublicationTiming,
        selected_lanes: Vec<CaptionLane>,
    },
    Incompatible {
        requested_mode: PublicationMode,
        selected_lanes: Vec<CaptionLane>,
        reason: PublicationIncompatibility,
        supported_modes: Vec<PublicationMode>,
    },
}

impl PublicationPlan {
    pub(crate) fn resolved_timing(&self) -> Option<ResolvedPublicationTiming> {
        match self {
            Self::Compatible { timing, .. } => Some(*timing),
            Self::Incompatible { .. } => None,
        }
    }

    pub(crate) fn incompatibility_code(&self) -> Option<&'static str> {
        match self {
            Self::Compatible { .. } => None,
            Self::Incompatible { reason, .. } => Some(match reason {
                PublicationIncompatibility::NoLanesSelected => "publication.no_lanes_selected",
                PublicationIncompatibility::LaneUnavailable { .. } => {
                    "publication.lane_unavailable"
                }
                PublicationIncompatibility::ModeUnsupported { .. } => {
                    "publication.mode_unsupported"
                }
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptionPipelinePlan {
    pub(crate) recognition: RecognitionCapabilityProfile,
    pub(crate) translation: Option<TranslationCapabilityProfile>,
    pub(crate) publication: PublicationPlan,
}

pub(crate) fn plan_caption_pipeline(config: &AppConfig) -> CaptionPipelinePlan {
    let recognition = recognition_capabilities(&config.recognition);
    let translation = active_translation(config);
    let mut available_lanes = recognition.lanes.clone();
    if let Some(profile) = &translation {
        available_lanes.extend(profile.lanes.iter().cloned());
    }
    let publication = plan_publication(
        &available_lanes,
        config.publication.mode,
        selected_lanes(config.publication.content),
    );
    CaptionPipelinePlan {
        recognition,
        translation,
        publication,
    }
}

fn active_translation(config: &AppConfig) -> Option<TranslationCapabilityProfile> {
    match config.publication.content {
        ContentSelection::SourceOnly => None,
        ContentSelection::TranslationOnly | ContentSelection::Bilingual => {
            config.translation.as_ref().map(translation_capabilities)
        }
    }
}

fn selected_lanes(content: ContentSelection) -> &'static [CaptionLane] {
    match content {
        ContentSelection::SourceOnly => &[CaptionLane::Source],
        ContentSelection::TranslationOnly => &[CaptionLane::Translation],
        ContentSelection::Bilingual => &[CaptionLane::Source, CaptionLane::Translation],
    }
}

fn translation_capabilities(translation: &TranslationConfig) -> TranslationCapabilityProfile {
    match translation.path {
        TranslationPath::OpenAiResponsesCompletedText => TranslationCapabilityProfile {
            path: translation.path,
            input_shape: TranslationInputShape::CompletedSourceSnapshots,
            lanes: vec![LaneCapabilities {
                lane: CaptionLane::Translation,
                updates: LaneUpdateBehavior::CompletedOnly,
                revisions: RevisionBehavior::AppendOnly,
            }],
        },
    }
}

pub(crate) fn resolve_caption_pipeline_start_timing(
    plan: &CaptionPipelinePlan,
) -> AppResult<ResolvedPublicationTiming> {
    resolve_caption_pipeline_timing(plan)
}

/// Resolves planner compatibility without composing selected Modules. The
/// desktop Start preflight prepares every active owner before Runtime begins.
pub(crate) fn resolve_caption_pipeline_timing(
    plan: &CaptionPipelinePlan,
) -> AppResult<ResolvedPublicationTiming> {
    plan.publication.resolved_timing().ok_or_else(|| {
        AppError::config(format!(
            "The selected recognition path and publication mode are incompatible ({}).",
            plan.publication
                .incompatibility_code()
                .unwrap_or("publication.incompatible")
        ))
    })
}

pub(crate) fn recognition_capabilities(
    recognition: &RecognitionConfig,
) -> RecognitionCapabilityProfile {
    match recognition.path {
        RecognitionPath::OpenAiGptTranscribe => profile(
            RecognitionPath::OpenAiGptTranscribe,
            RecognitionInputShape::ContinuousAudioFrames,
            CaptionBoundaryOwner::Application,
            CaptionUnitBehavior::UnitBased,
            RevisionBehavior::AppendOnly,
            LaneUpdateBehavior::CompletedOnly,
        ),
        RecognitionPath::OpenAiGptLiveTranscribe => profile(
            RecognitionPath::OpenAiGptLiveTranscribe,
            RecognitionInputShape::ContinuousAudioFrames,
            CaptionBoundaryOwner::Application,
            CaptionUnitBehavior::UnitBased,
            RevisionBehavior::RevisableFullSnapshot,
            LaneUpdateBehavior::OngoingAndCompleted,
        ),
    }
}

pub(crate) fn plan_publication(
    available_lanes: &[LaneCapabilities],
    requested_mode: PublicationMode,
    selected_lanes: &[CaptionLane],
) -> PublicationPlan {
    let selected_lanes = deduplicate_lanes(selected_lanes);
    if selected_lanes.is_empty() {
        return incompatible(
            requested_mode,
            selected_lanes,
            PublicationIncompatibility::NoLanesSelected,
            Vec::new(),
        );
    }

    let unavailable_lanes = selected_lanes
        .iter()
        .copied()
        .filter(|lane| !available_lanes.iter().any(|item| item.lane == *lane))
        .collect::<Vec<_>>();
    if !unavailable_lanes.is_empty() {
        return incompatible(
            requested_mode,
            selected_lanes,
            PublicationIncompatibility::LaneUnavailable {
                lanes: unavailable_lanes,
            },
            Vec::new(),
        );
    }

    let unsupported_for_request =
        unsupported_lanes(available_lanes, requested_mode, &selected_lanes);
    if unsupported_for_request.is_empty() {
        return PublicationPlan::Compatible {
            mode: requested_mode,
            timing: resolved_timing(requested_mode),
            selected_lanes,
        };
    }

    let supported_modes = [PublicationMode::Completed, PublicationMode::Live]
        .into_iter()
        .filter(|mode| unsupported_lanes(available_lanes, *mode, &selected_lanes).is_empty())
        .collect();
    incompatible(
        requested_mode,
        selected_lanes,
        PublicationIncompatibility::ModeUnsupported {
            lanes: unsupported_for_request,
        },
        supported_modes,
    )
}

fn resolved_timing(mode: PublicationMode) -> ResolvedPublicationTiming {
    match mode {
        PublicationMode::Completed => ResolvedPublicationTiming::Completed,
        PublicationMode::Live => ResolvedPublicationTiming::LiveUnit {
            observation_window_ms: LIVE_OBSERVATION_MILLIS,
        },
    }
}

fn profile(
    path: RecognitionPath,
    input_shape: RecognitionInputShape,
    caption_boundary_owner: CaptionBoundaryOwner,
    unit_behavior: CaptionUnitBehavior,
    revision_behavior: RevisionBehavior,
    source_updates: LaneUpdateBehavior,
) -> RecognitionCapabilityProfile {
    RecognitionCapabilityProfile {
        path,
        input_shape,
        caption_boundary_owner,
        unit_behavior,
        lanes: vec![LaneCapabilities {
            lane: CaptionLane::Source,
            updates: source_updates,
            revisions: revision_behavior,
        }],
    }
}

fn deduplicate_lanes(selected_lanes: &[CaptionLane]) -> Vec<CaptionLane> {
    selected_lanes
        .iter()
        .copied()
        .fold(Vec::new(), |mut lanes, lane| {
            if !lanes.contains(&lane) {
                lanes.push(lane);
            }
            lanes
        })
}

fn unsupported_lanes(
    available_lanes: &[LaneCapabilities],
    mode: PublicationMode,
    selected_lanes: &[CaptionLane],
) -> Vec<CaptionLane> {
    selected_lanes
        .iter()
        .copied()
        .filter(|lane| {
            available_lanes
                .iter()
                .find(|item| item.lane == *lane)
                .is_none_or(|item| !item.updates.supports(mode))
        })
        .collect()
}

fn incompatible(
    requested_mode: PublicationMode,
    selected_lanes: Vec<CaptionLane>,
    reason: PublicationIncompatibility,
    supported_modes: Vec<PublicationMode>,
) -> PublicationPlan {
    PublicationPlan::Incompatible {
        requested_mode,
        selected_lanes,
        reason,
        supported_modes,
    }
}

#[cfg(test)]
#[path = "caption_pipeline_tests.rs"]
mod tests;
