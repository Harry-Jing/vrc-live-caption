//! Normalized caption contract and authoritative cross-generation aggregate.

mod aggregate;
mod contract;

pub(crate) use aggregate::{CaptionAggregateChange, CaptionAggregateStore, CaptionAggregateUpdate};
pub(crate) use contract::{CaptionAggregateSnapshot, CaptionLane, CaptionSnapshot, CaptionState};

#[cfg(test)]
pub(crate) use contract::{
    ActiveCaptionStream, CAPTION_AGGREGATE_CONTRACT_VERSION, OpenSourceUnit,
};
