//! Provider-path capabilities and publication compatibility planning.

use crate::caption_session::CaptionLane;
use crate::config::{AppConfig, PublicationMode, SttConfig, SttProvider};
use serde::Serialize;

pub(crate) const MOCK_BOUNDED_MODEL: &str = "mock-bounded";
pub(crate) const MOCK_ONGOING_COMPLETED_MODEL: &str = "mock-ongoing-completed";
pub(crate) const MOCK_ONGOING_ONLY_MODEL: &str = "mock-ongoing-only";
pub(crate) const LIVE_OBSERVATION_MILLIS: u64 = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RecognitionPath {
    OpenAiBounded,
    MockBounded,
    MockOngoingCompleted,
    MockOngoingOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RecognitionInputShape {
    CompletedAudioUnits,
    ContinuousAudioFrames,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum BoundaryOwner {
    Application,
    Provider,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CaptionUnitBehavior {
    UnitBased,
    Unitless,
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
    OngoingOnly,
}

impl LaneUpdateBehavior {
    fn supports(self, mode: PublicationMode) -> bool {
        match mode {
            PublicationMode::Completed => {
                matches!(self, Self::CompletedOnly | Self::OngoingAndCompleted)
            }
            PublicationMode::Live => {
                matches!(self, Self::OngoingAndCompleted | Self::OngoingOnly)
            }
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
    pub(crate) boundary_owner: BoundaryOwner,
    pub(crate) unit_behavior: CaptionUnitBehavior,
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
#[serde(tag = "policy", rename_all = "camelCase")]
pub(crate) enum ResolvedPublicationPolicy {
    Completed,
    LiveUnit { observation_window_ms: u64 },
    LiveUnitless { first_non_empty_delay_ms: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub(crate) enum PublicationPlan {
    Ready {
        mode: PublicationMode,
        policy: ResolvedPublicationPolicy,
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
    pub(crate) fn resolved_policy(&self) -> Option<ResolvedPublicationPolicy> {
        match self {
            Self::Ready { policy, .. } => Some(*policy),
            Self::Incompatible { .. } => None,
        }
    }

    pub(crate) fn incompatibility_code(&self) -> Option<&'static str> {
        match self {
            Self::Ready { .. } => None,
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
pub(crate) struct RuntimePlanSnapshot {
    pub(crate) recognition: RecognitionCapabilityProfile,
    pub(crate) publication: PublicationPlan,
}

pub(crate) fn plan_runtime(config: &AppConfig) -> RuntimePlanSnapshot {
    let recognition = recognition_capabilities(&config.stt);
    let publication = plan_publication(
        &recognition,
        config.publication.mode,
        &[CaptionLane::Source],
    );
    RuntimePlanSnapshot {
        recognition,
        publication,
    }
}

pub(crate) fn recognition_capabilities(stt: &SttConfig) -> RecognitionCapabilityProfile {
    match stt.provider {
        SttProvider::OpenAi => profile(
            RecognitionPath::OpenAiBounded,
            RecognitionInputShape::CompletedAudioUnits,
            BoundaryOwner::Application,
            CaptionUnitBehavior::UnitBased,
            RevisionBehavior::AppendOnly,
            LaneUpdateBehavior::CompletedOnly,
        ),
        SttProvider::Mock => match stt.model.as_str() {
            MOCK_BOUNDED_MODEL => profile(
                RecognitionPath::MockBounded,
                RecognitionInputShape::CompletedAudioUnits,
                BoundaryOwner::Application,
                CaptionUnitBehavior::UnitBased,
                RevisionBehavior::AppendOnly,
                LaneUpdateBehavior::CompletedOnly,
            ),
            MOCK_ONGOING_ONLY_MODEL => profile(
                RecognitionPath::MockOngoingOnly,
                RecognitionInputShape::ContinuousAudioFrames,
                BoundaryOwner::None,
                CaptionUnitBehavior::Unitless,
                RevisionBehavior::RevisableFullSnapshot,
                LaneUpdateBehavior::OngoingOnly,
            ),
            MOCK_ONGOING_COMPLETED_MODEL => mock_ongoing_completed_profile(),
            _ => mock_ongoing_completed_profile(),
        },
    }
}

fn mock_ongoing_completed_profile() -> RecognitionCapabilityProfile {
    profile(
        RecognitionPath::MockOngoingCompleted,
        RecognitionInputShape::ContinuousAudioFrames,
        BoundaryOwner::Provider,
        CaptionUnitBehavior::UnitBased,
        RevisionBehavior::RevisableFullSnapshot,
        LaneUpdateBehavior::OngoingAndCompleted,
    )
}

pub(crate) fn plan_publication(
    capabilities: &RecognitionCapabilityProfile,
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
        .filter(|lane| !capabilities.lanes.iter().any(|item| item.lane == *lane))
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

    let unsupported_for_request = unsupported_lanes(capabilities, requested_mode, &selected_lanes);
    if unsupported_for_request.is_empty() {
        return PublicationPlan::Ready {
            mode: requested_mode,
            policy: resolved_policy(capabilities, requested_mode),
            selected_lanes,
        };
    }

    let supported_modes = [PublicationMode::Completed, PublicationMode::Live]
        .into_iter()
        .filter(|mode| unsupported_lanes(capabilities, *mode, &selected_lanes).is_empty())
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

fn resolved_policy(
    capabilities: &RecognitionCapabilityProfile,
    mode: PublicationMode,
) -> ResolvedPublicationPolicy {
    match (mode, capabilities.unit_behavior) {
        (PublicationMode::Completed, _) => ResolvedPublicationPolicy::Completed,
        (PublicationMode::Live, CaptionUnitBehavior::UnitBased) => {
            ResolvedPublicationPolicy::LiveUnit {
                observation_window_ms: LIVE_OBSERVATION_MILLIS,
            }
        }
        (PublicationMode::Live, CaptionUnitBehavior::Unitless) => {
            ResolvedPublicationPolicy::LiveUnitless {
                first_non_empty_delay_ms: LIVE_OBSERVATION_MILLIS,
            }
        }
    }
}

fn profile(
    path: RecognitionPath,
    input_shape: RecognitionInputShape,
    boundary_owner: BoundaryOwner,
    unit_behavior: CaptionUnitBehavior,
    revision_behavior: RevisionBehavior,
    source_updates: LaneUpdateBehavior,
) -> RecognitionCapabilityProfile {
    RecognitionCapabilityProfile {
        path,
        input_shape,
        boundary_owner,
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
    capabilities: &RecognitionCapabilityProfile,
    mode: PublicationMode,
    selected_lanes: &[CaptionLane],
) -> Vec<CaptionLane> {
    selected_lanes
        .iter()
        .copied()
        .filter(|lane| {
            capabilities
                .lanes
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
#[path = "capability_planner_tests.rs"]
mod tests;
