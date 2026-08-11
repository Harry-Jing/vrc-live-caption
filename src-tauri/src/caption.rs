//! Normalized caption contract and authoritative cross-generation aggregate.

mod aggregate;
mod contract;

pub(crate) use aggregate::{CaptionAggregateChange, CaptionAggregateStore, CaptionAggregateUpdate};
pub(crate) use contract::{
    CaptionAggregateSnapshotV2, CaptionLane, CaptionSnapshotV2, CaptionState,
};

#[cfg(test)]
pub(crate) use contract::{
    ActiveCaptionStreamV2, CAPTION_AGGREGATE_CONTRACT_VERSION, OpenSourceUnitV2,
};
