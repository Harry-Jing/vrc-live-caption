//! Application-owned normalized caption aggregate state.

use std::cmp::Reverse;
use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex};

use super::contract::{
    ActiveCaptionStream, CAPTION_AGGREGATE_CONTRACT_VERSION, CaptionAggregateSnapshot, CaptionLane,
    CaptionSnapshot, CaptionState, OpenSourceUnit,
};
use crate::error::{AppError, AppResult};

const COMPLETED_UNIT_LIMIT: usize = 5;
// Unit identity is generation + stream + unit ID. Keep a bounded recent per-lane
// replay guard so terminal duplicates cannot re-enter history after trimming.
const TERMINAL_LANE_REPLAY_LIMIT: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
struct CaptionUnitKey {
    generation: u64,
    stream_id: String,
    unit_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CaptionLaneKey {
    unit: CaptionUnitKey,
    lane: CaptionLane,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CompletedSourceKey {
    unit: CaptionUnitKey,
    revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CaptionAggregateUpdate {
    pub(crate) snapshot: CaptionAggregateSnapshot,
    pub(crate) change: CaptionAggregateChange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CaptionAggregateChange {
    SourceUnitOpened(OpenSourceUnit),
    SourceUnitAborted { unit_id: String },
    CaptionAccepted(CaptionSnapshot),
}

#[derive(Default)]
struct CaptionAggregateState {
    generation_high_watermark: u64,
    snapshot_revision: u64,
    // Application-only ordering metadata. The wire contract conveys this order
    // through its `captions` array and does not expose another clock-like field.
    next_unit_ordinal: u64,
    active_stream: Option<ActiveCaptionStream>,
    open_source_units: Vec<OpenSourceUnit>,
    captions: Vec<CaptionSnapshot>,
    unit_ordinals: Vec<(CaptionUnitKey, u64)>,
    completed_units: VecDeque<CaptionUnitKey>,
    pinned_sources: Vec<CompletedSourceKey>,
    recent_terminal_lanes: VecDeque<CaptionLaneKey>,
}

#[derive(Clone, Default)]
pub(crate) struct CaptionAggregateStore {
    state: Arc<Mutex<CaptionAggregateState>>,
}

pub(crate) struct ReservedCompletedSource {
    store: CaptionAggregateStore,
    source: CaptionSnapshot,
    key: Option<CompletedSourceKey>,
}

impl fmt::Debug for ReservedCompletedSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReservedCompletedSource")
            .field("generation", &self.source.generation)
            .field("stream_id", &self.source.stream_id)
            .field("unit_id", &self.source.unit_id)
            .field("revision", &self.source.revision)
            .finish_non_exhaustive()
    }
}

impl Drop for ReservedCompletedSource {
    fn drop(&mut self) {
        let Some(key) = self.key.take() else {
            return;
        };
        let Ok(mut state) = self.store.state.lock() else {
            return;
        };
        CaptionAggregateStore::release_reserved_source(&mut state, &key);
    }
}

impl ReservedCompletedSource {
    pub(crate) fn source(&self) -> &CaptionSnapshot {
        &self.source
    }

    pub(crate) fn complete_translation(
        mut self,
        text: String,
        language: String,
        timestamp_ms: u64,
    ) -> AppResult<Option<CaptionAggregateUpdate>> {
        let mut state = self.store.lock()?;
        let Some(key) = self.key.as_ref() else {
            return Ok(None);
        };
        let source_is_current =
            CaptionAggregateStore::matches_active(&state, key.unit.generation, &key.unit.stream_id)
                && state.pinned_sources.contains(key)
                && state.captions.iter().any(|caption| caption == &self.source);
        let translation_lane = CaptionLaneKey {
            unit: key.unit.clone(),
            lane: CaptionLane::Translation,
        };
        if !source_is_current || state.recent_terminal_lanes.contains(&translation_lane) {
            CaptionAggregateStore::release_reserved_source(&mut state, key);
            self.key = None;
            return Ok(None);
        }

        let translation = CaptionSnapshot {
            generation: key.unit.generation,
            stream_id: key.unit.stream_id.clone(),
            unit_id: Some(key.unit.unit_id.clone()),
            lane: CaptionLane::Translation,
            revision: 1,
            text,
            state: CaptionState::Completed,
            language: Some(language),
            source_ref: Some(super::contract::SourceSnapshotRef {
                generation: key.unit.generation,
                stream_id: key.unit.stream_id.clone(),
                unit_id: key.unit.unit_id.clone(),
                revision: key.revision,
            }),
            unit_started_at_ms: self.source.unit_started_at_ms,
            timestamp_ms,
        };
        state.captions.insert(0, translation.clone());
        CaptionAggregateStore::record_terminal_lane(&mut state, translation_lane);
        state.pinned_sources.retain(|pinned| pinned != key);
        self.key = None;
        CaptionAggregateStore::trim_completed_units(&mut state);
        CaptionAggregateStore::sort_captions_by_unit_order(&mut state);
        CaptionAggregateStore::advance_revision(&mut state);

        Ok(Some(CaptionAggregateUpdate {
            snapshot: CaptionAggregateStore::snapshot_from(&state),
            change: CaptionAggregateChange::CaptionAccepted(translation),
        }))
    }
}

impl CaptionAggregateStore {
    pub(crate) fn begin_generation(&self, generation: u64) -> AppResult<CaptionAggregateSnapshot> {
        let mut state = self.lock()?;
        if generation <= state.generation_high_watermark {
            return Ok(Self::snapshot_from(&state));
        }

        state.generation_high_watermark = generation;
        state.pinned_sources.clear();
        state.active_stream = Some(ActiveCaptionStream {
            generation,
            stream_id: format!("recognition-{generation}-1"),
        });
        state.open_source_units.clear();
        state.recent_terminal_lanes.clear();
        state
            .captions
            .retain(|caption| caption.state == CaptionState::Completed);
        state.next_unit_ordinal = 0;
        Self::trim_completed_units(&mut state);
        Self::retain_caption_unit_ordinals(&mut state);
        Self::advance_revision(&mut state);

        Ok(Self::snapshot_from(&state))
    }

    pub(crate) fn start_unit(
        &self,
        generation: u64,
        stream_id: &str,
        unit_id: String,
        started_at_ms: u64,
    ) -> AppResult<Option<CaptionAggregateUpdate>> {
        let mut state = self.lock()?;
        let unit_key = CaptionUnitKey {
            generation,
            stream_id: stream_id.to_string(),
            unit_id: unit_id.clone(),
        };
        if !Self::matches_active(&state, generation, stream_id)
            || state.recent_terminal_lanes.contains(&CaptionLaneKey {
                unit: unit_key.clone(),
                lane: CaptionLane::Source,
            })
            || state
                .open_source_units
                .iter()
                .any(|unit| unit.unit_id == unit_id)
            || state.completed_units.contains(&unit_key)
        {
            return Ok(None);
        }

        let unit_ordinal = state.next_unit_ordinal.checked_add(1).ok_or_else(|| {
            AppError::state("Caption unit order was exhausted for the active generation.")
        })?;
        state.next_unit_ordinal = unit_ordinal;
        state.unit_ordinals.push((unit_key, unit_ordinal));
        let opened_source_unit = OpenSourceUnit {
            unit_id,
            started_at_ms,
        };
        state.open_source_units.push(opened_source_unit.clone());
        Self::advance_revision(&mut state);

        Ok(Some(CaptionAggregateUpdate {
            snapshot: Self::snapshot_from(&state),
            change: CaptionAggregateChange::SourceUnitOpened(opened_source_unit),
        }))
    }

    pub(crate) fn accept_caption(
        &self,
        caption: CaptionSnapshot,
    ) -> AppResult<Option<CaptionAggregateUpdate>> {
        if caption.lane == CaptionLane::Translation {
            return Ok(None);
        }

        let mut state = self.lock()?;
        Ok(Self::accept_caption_locked(&mut state, caption, false).map(|(update, _)| update))
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "GitHub issue #11 integrates the Phase 5 Translation core with Runtime in its next child slice."
        )
    )]
    pub(crate) fn accept_completed_source_for_translation(
        &self,
        caption: CaptionSnapshot,
    ) -> AppResult<Option<(CaptionAggregateUpdate, ReservedCompletedSource)>> {
        if caption.lane != CaptionLane::Source || caption.state != CaptionState::Completed {
            return Ok(None);
        }

        let source = caption.clone();
        let mut state = self.lock()?;
        let Some((update, Some(key))) = Self::accept_caption_locked(&mut state, caption, true)
        else {
            return Ok(None);
        };
        drop(state);

        Ok(Some((
            update,
            ReservedCompletedSource {
                store: self.clone(),
                source,
                key: Some(key),
            },
        )))
    }

    fn accept_caption_locked(
        state: &mut CaptionAggregateState,
        caption: CaptionSnapshot,
        reserve_completed_source: bool,
    ) -> Option<(CaptionAggregateUpdate, Option<CompletedSourceKey>)> {
        if !Self::matches_active(state, caption.generation, &caption.stream_id)
            || caption.revision == 0
            || (caption.state == CaptionState::Completed && caption.unit_id.is_none())
            || !Self::source_reference_is_valid(state, &caption)
        {
            return None;
        }

        let unit_key = caption.unit_id.as_ref().map(|unit_id| CaptionUnitKey {
            generation: caption.generation,
            stream_id: caption.stream_id.clone(),
            unit_id: unit_id.clone(),
        });
        let lane_key = unit_key.as_ref().map(|unit| CaptionLaneKey {
            unit: unit.clone(),
            lane: caption.lane,
        });
        if lane_key
            .as_ref()
            .is_some_and(|lane_key| state.recent_terminal_lanes.contains(lane_key))
        {
            return None;
        }

        let existing = state.captions.iter().position(|current| {
            current.generation == caption.generation
                && current.stream_id == caption.stream_id
                && current.unit_id == caption.unit_id
                && current.lane == caption.lane
        });
        if existing.is_some_and(|index| {
            let current = &state.captions[index];
            current.state == CaptionState::Completed || current.revision >= caption.revision
        }) {
            return None;
        }

        if let Some(unit_key) = unit_key.as_ref()
            && !state
                .open_source_units
                .iter()
                .any(|unit| unit.unit_id == unit_key.unit_id)
            && !(caption.lane == CaptionLane::Translation
                && state.completed_units.contains(unit_key))
        {
            return None;
        }

        if let Some(index) = existing {
            state.captions.remove(index);
        }

        let accepted_caption = caption.clone();
        let mut reservation_key = None;
        if caption.state == CaptionState::Completed {
            let unit_id = caption.unit_id.clone()?;
            if caption.lane == CaptionLane::Source {
                state
                    .open_source_units
                    .retain(|unit| unit.unit_id != unit_id);
            }
            state.captions.insert(0, caption.clone());
            let completed_unit = CaptionUnitKey {
                generation: caption.generation,
                stream_id: caption.stream_id.clone(),
                unit_id,
            };
            Self::record_terminal_lane(
                state,
                CaptionLaneKey {
                    unit: completed_unit.clone(),
                    lane: caption.lane,
                },
            );
            if caption.lane == CaptionLane::Source {
                state
                    .completed_units
                    .retain(|current| current != &completed_unit);
                state.completed_units.push_front(completed_unit.clone());
                if reserve_completed_source {
                    let key = CompletedSourceKey {
                        unit: completed_unit,
                        revision: caption.revision,
                    };
                    state.pinned_sources.push(key.clone());
                    reservation_key = Some(key);
                }
                Self::trim_completed_units(state);
            }
        } else {
            state.captions.insert(0, caption);
        }
        Self::sort_captions_by_unit_order(state);
        Self::advance_revision(state);

        Some((
            CaptionAggregateUpdate {
                snapshot: Self::snapshot_from(state),
                change: CaptionAggregateChange::CaptionAccepted(accepted_caption),
            },
            reservation_key,
        ))
    }

    pub(crate) fn abort_source_unit(
        &self,
        generation: u64,
        stream_id: &str,
        unit_id: &str,
    ) -> AppResult<Option<CaptionAggregateUpdate>> {
        let mut state = self.lock()?;
        if !Self::matches_active(&state, generation, stream_id)
            || !state
                .open_source_units
                .iter()
                .any(|unit| unit.unit_id == unit_id)
        {
            return Ok(None);
        }

        state
            .open_source_units
            .retain(|unit| unit.unit_id != unit_id);
        state.captions.retain(|caption| {
            caption.state == CaptionState::Completed
                || caption.generation != generation
                || caption.stream_id != stream_id
                || caption.unit_id.as_deref() != Some(unit_id)
        });
        Self::record_terminal_lane(
            &mut state,
            CaptionLaneKey {
                unit: CaptionUnitKey {
                    generation,
                    stream_id: stream_id.to_string(),
                    unit_id: unit_id.to_string(),
                },
                lane: CaptionLane::Source,
            },
        );
        Self::retain_caption_unit_ordinals(&mut state);
        Self::advance_revision(&mut state);

        Ok(Some(CaptionAggregateUpdate {
            snapshot: Self::snapshot_from(&state),
            change: CaptionAggregateChange::SourceUnitAborted {
                unit_id: unit_id.to_string(),
            },
        }))
    }

    pub(crate) fn close_generation(
        &self,
        generation: u64,
    ) -> AppResult<Option<CaptionAggregateSnapshot>> {
        let mut state = self.lock()?;
        if state.active_stream.as_ref().map(|active| active.generation) != Some(generation) {
            return Ok(None);
        }

        state.active_stream = None;
        state
            .pinned_sources
            .retain(|source| source.unit.generation != generation);
        state.open_source_units.clear();
        state
            .captions
            .retain(|caption| caption.state == CaptionState::Completed);
        Self::trim_completed_units(&mut state);
        Self::retain_caption_unit_ordinals(&mut state);
        Self::advance_revision(&mut state);

        Ok(Some(Self::snapshot_from(&state)))
    }

    pub(crate) fn snapshot(&self) -> AppResult<CaptionAggregateSnapshot> {
        let state = self.lock()?;
        Ok(Self::snapshot_from(&state))
    }

    fn lock(&self) -> AppResult<std::sync::MutexGuard<'_, CaptionAggregateState>> {
        self.state
            .lock()
            .map_err(|_| AppError::state("Caption aggregate state lock was poisoned."))
    }

    fn matches_active(state: &CaptionAggregateState, generation: u64, stream_id: &str) -> bool {
        state
            .active_stream
            .as_ref()
            .is_some_and(|active| active.generation == generation && active.stream_id == stream_id)
    }

    fn source_reference_is_valid(state: &CaptionAggregateState, caption: &CaptionSnapshot) -> bool {
        match (caption.lane, caption.source_ref.as_ref()) {
            (CaptionLane::Source, None) => true,
            (CaptionLane::Source, Some(_)) | (CaptionLane::Translation, None) => false,
            (CaptionLane::Translation, Some(source)) => {
                caption.unit_id.as_deref() == Some(source.unit_id.as_str())
                    && caption.generation == source.generation
                    && caption.stream_id == source.stream_id
                    && state.captions.iter().any(|current| {
                        current.lane == CaptionLane::Source
                            && current.state == CaptionState::Completed
                            && current.generation == source.generation
                            && current.stream_id == source.stream_id
                            && current.unit_id.as_deref() == Some(source.unit_id.as_str())
                            && current.revision == source.revision
                    })
            }
        }
    }

    fn advance_revision(state: &mut CaptionAggregateState) {
        state.snapshot_revision = state.snapshot_revision.saturating_add(1);
    }

    fn snapshot_from(state: &CaptionAggregateState) -> CaptionAggregateSnapshot {
        CaptionAggregateSnapshot {
            contract_version: CAPTION_AGGREGATE_CONTRACT_VERSION,
            snapshot_revision: state.snapshot_revision,
            active_stream: state.active_stream.clone(),
            open_source_units: state.open_source_units.clone(),
            captions: state.captions.clone(),
        }
    }

    fn trim_completed_units(state: &mut CaptionAggregateState) {
        while state
            .completed_units
            .iter()
            .filter(|unit| !Self::unit_is_pinned(state, unit))
            .count()
            > COMPLETED_UNIT_LIMIT
        {
            let expired_index = state
                .completed_units
                .iter()
                .enumerate()
                .filter(|(_, unit)| !Self::unit_is_pinned(state, unit))
                .min_by_key(|(_, unit)| Self::unit_order(state, unit))
                .map(|(index, _)| index);
            if let Some(expired) =
                expired_index.and_then(|index| state.completed_units.remove(index))
            {
                state.captions.retain(|caption| {
                    caption.generation != expired.generation
                        || caption.stream_id != expired.stream_id
                        || caption.unit_id.as_deref() != Some(expired.unit_id.as_str())
                });
            }
        }
        Self::retain_caption_unit_ordinals(state);
    }

    fn unit_is_pinned(state: &CaptionAggregateState, unit: &CaptionUnitKey) -> bool {
        state
            .pinned_sources
            .iter()
            .any(|source| &source.unit == unit)
    }

    fn release_reserved_source(state: &mut CaptionAggregateState, key: &CompletedSourceKey) {
        let caption_count = state.captions.len();
        state.pinned_sources.retain(|pinned| pinned != key);
        Self::trim_completed_units(state);
        if state.captions.len() != caption_count {
            Self::advance_revision(state);
        }
    }

    fn unit_order(state: &CaptionAggregateState, unit: &CaptionUnitKey) -> (u64, u64) {
        let ordinal = state
            .unit_ordinals
            .iter()
            .find_map(|(key, ordinal)| (key == unit).then_some(*ordinal))
            .unwrap_or_default();
        (unit.generation, ordinal)
    }

    fn sort_captions_by_unit_order(state: &mut CaptionAggregateState) {
        let unit_ordinals = &state.unit_ordinals;
        state.captions.sort_by_key(|caption| {
            let unit_ordinal = match caption.unit_id.as_deref() {
                None => u64::MAX,
                Some(unit_id) => unit_ordinals
                    .iter()
                    .find_map(|(key, ordinal)| {
                        (key.generation == caption.generation
                            && key.stream_id == caption.stream_id
                            && key.unit_id == unit_id)
                            .then_some(*ordinal)
                    })
                    .unwrap_or_default(),
            };
            let lane_order = match caption.lane {
                CaptionLane::Source => 0_u8,
                CaptionLane::Translation => 1_u8,
            };
            (Reverse((caption.generation, unit_ordinal)), lane_order)
        });
    }

    fn retain_caption_unit_ordinals(state: &mut CaptionAggregateState) {
        let captions = &state.captions;
        let active = state.active_stream.as_ref();
        let open_source_units = &state.open_source_units;
        state.unit_ordinals.retain(|(key, _)| {
            captions.iter().any(|caption| {
                caption.generation == key.generation
                    && caption.stream_id == key.stream_id
                    && caption.unit_id.as_deref() == Some(key.unit_id.as_str())
            }) || active.is_some_and(|stream| {
                stream.generation == key.generation
                    && stream.stream_id == key.stream_id
                    && open_source_units
                        .iter()
                        .any(|unit| unit.unit_id == key.unit_id)
            })
        });
    }

    fn record_terminal_lane(state: &mut CaptionAggregateState, lane: CaptionLaneKey) {
        state
            .recent_terminal_lanes
            .retain(|current| current != &lane);
        state.recent_terminal_lanes.push_back(lane);
        while state.recent_terminal_lanes.len() > TERMINAL_LANE_REPLAY_LIMIT {
            state.recent_terminal_lanes.pop_front();
        }
    }
}

#[cfg(test)]
#[path = "aggregate_tests.rs"]
mod tests;
