//! Rust serialization types for the versioned Caption Aggregate wire contract.
//!
//! This module defines payload shape only. Aggregate admission, revision
//! ordering, retention, and snapshot construction live in `aggregate`.

use serde::{Deserialize, Serialize};

pub(crate) const CAPTION_AGGREGATE_CONTRACT_VERSION: u32 = 2;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CaptionAggregateSnapshot {
    pub(crate) contract_version: u32,
    pub(crate) snapshot_revision: u64,
    pub(crate) active_stream: Option<ActiveCaptionStream>,
    pub(crate) open_source_units: Vec<OpenSourceUnit>,
    pub(crate) captions: Vec<CaptionSnapshot>,
    pub(crate) translation_units: Vec<TranslationUnitSnapshot>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ActiveCaptionStream {
    pub(crate) generation: u64,
    pub(crate) stream_id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OpenSourceUnit {
    pub(crate) unit_id: String,
    pub(crate) started_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CaptionSnapshot {
    pub(crate) generation: u64,
    pub(crate) stream_id: String,
    pub(crate) unit_id: Option<String>,
    pub(crate) lane: CaptionLane,
    pub(crate) revision: u64,
    pub(crate) text: String,
    pub(crate) state: CaptionState,
    pub(crate) language: Option<String>,
    pub(crate) source_ref: Option<SourceSnapshotRef>,
    pub(crate) unit_started_at_ms: Option<u64>,
    pub(crate) timestamp_ms: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SourceSnapshotRef {
    pub(crate) generation: u64,
    pub(crate) stream_id: String,
    pub(crate) unit_id: String,
    pub(crate) revision: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) enum TranslationFailureReason {
    #[serde(rename = "translation.provider_authentication_failed")]
    ProviderAuthenticationFailed,
    #[serde(rename = "translation.provider_permission_denied")]
    ProviderPermissionDenied,
    #[serde(rename = "translation.provider_invalid_request")]
    ProviderInvalidRequest,
    #[serde(rename = "translation.provider_rate_limited")]
    ProviderRateLimited,
    #[serde(rename = "translation.provider_usage_limit")]
    ProviderUsageLimit,
    #[serde(rename = "translation.provider_unavailable")]
    ProviderUnavailable,
    #[serde(rename = "translation.invalid_output")]
    InvalidOutput,
    #[serde(rename = "translation.deadline_exceeded")]
    DeadlineExceeded,
    #[serde(rename = "translation.backpressure")]
    Backpressure,
    #[serde(rename = "translation.source_too_large")]
    SourceTooLarge,
    #[serde(rename = "translation.stopped")]
    Stopped,
    #[serde(rename = "translation.failed")]
    Failed,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum TranslationUnitSnapshot {
    Pending {
        source_ref: SourceSnapshotRef,
    },
    Completed {
        source_ref: SourceSnapshotRef,
    },
    Failed {
        source_ref: SourceSnapshotRef,
        reason_code: TranslationFailureReason,
    },
}

impl TranslationUnitSnapshot {
    pub(crate) fn source_ref(&self) -> &SourceSnapshotRef {
        match self {
            Self::Pending { source_ref }
            | Self::Completed { source_ref }
            | Self::Failed { source_ref, .. } => source_ref,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CaptionLane {
    Source,
    Translation,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CaptionState {
    Ongoing,
    Completed,
}

#[cfg(test)]
#[path = "contract_tests.rs"]
mod tests;
