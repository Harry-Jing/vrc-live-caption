//! Normalized caption contract and authoritative cross-generation aggregate.

mod aggregate;
mod contract;

pub(crate) use aggregate::{
    CaptionAggregateChange, CaptionAggregateStore, CaptionAggregateUpdate, ReservedCompletedSource,
};
pub(crate) use contract::{
    CaptionAggregateSnapshot, CaptionLane, CaptionSnapshot, CaptionState, TranslationFailureReason,
};

#[cfg(test)]
pub(crate) use contract::{
    ActiveCaptionStream, CAPTION_AGGREGATE_CONTRACT_VERSION, OpenSourceUnit, SourceSnapshotRef,
    TranslationUnitSnapshot,
};
