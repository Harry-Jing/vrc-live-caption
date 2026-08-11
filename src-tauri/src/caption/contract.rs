use serde::{Deserialize, Serialize};

pub(crate) const CAPTION_AGGREGATE_CONTRACT_VERSION: u32 = 2;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CaptionAggregateSnapshotV2 {
    pub(crate) contract_version: u32,
    pub(crate) snapshot_revision: u64,
    pub(crate) active_stream: Option<ActiveCaptionStreamV2>,
    pub(crate) open_source_units: Vec<OpenSourceUnitV2>,
    pub(crate) captions: Vec<CaptionSnapshotV2>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ActiveCaptionStreamV2 {
    pub(crate) generation: u64,
    pub(crate) stream_id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OpenSourceUnitV2 {
    pub(crate) unit_id: String,
    pub(crate) started_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CaptionSnapshotV2 {
    pub(crate) generation: u64,
    pub(crate) stream_id: String,
    pub(crate) unit_id: Option<String>,
    pub(crate) lane: CaptionLane,
    pub(crate) revision: u64,
    pub(crate) text: String,
    pub(crate) state: CaptionState,
    pub(crate) language: Option<String>,
    pub(crate) source_ref: Option<SourceSnapshotRefV2>,
    pub(crate) unit_started_at_ms: Option<u64>,
    pub(crate) timestamp_ms: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SourceSnapshotRefV2 {
    pub(crate) generation: u64,
    pub(crate) stream_id: String,
    pub(crate) unit_id: String,
    pub(crate) revision: u64,
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
