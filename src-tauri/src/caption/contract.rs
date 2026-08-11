use serde::{Deserialize, Serialize};

pub(crate) const CAPTION_AGGREGATE_CONTRACT_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CaptionAggregateSnapshot {
    pub(crate) contract_version: u32,
    pub(crate) snapshot_revision: u64,
    pub(crate) active_stream: Option<ActiveCaptionStream>,
    pub(crate) open_source_units: Vec<OpenSourceUnit>,
    pub(crate) captions: Vec<CaptionSnapshot>,
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
