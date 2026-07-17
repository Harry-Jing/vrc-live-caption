//! Backend-owned normalized caption-session state.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::error::{AppError, AppResult};

pub(crate) const CAPTION_SESSION_CONTRACT_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CaptionSessionSnapshotV1 {
    pub(crate) contract_version: u32,
    pub(crate) snapshot_revision: u64,
    pub(crate) active: Option<CaptionSessionActiveV1>,
    pub(crate) active_units: Vec<CaptionActiveUnitV1>,
    pub(crate) captions: Vec<CaptionSnapshotV1>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CaptionSessionActiveV1 {
    pub(crate) generation: u64,
    pub(crate) stream_id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CaptionActiveUnitV1 {
    pub(crate) unit_id: String,
    pub(crate) started_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CaptionSnapshotV1 {
    pub(crate) generation: u64,
    pub(crate) stream_id: String,
    pub(crate) unit_id: Option<String>,
    pub(crate) lane: CaptionLane,
    pub(crate) revision: u64,
    pub(crate) text: String,
    pub(crate) state: CaptionState,
    pub(crate) language: Option<String>,
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) unit_started_at_ms: Option<u64>,
    pub(crate) timestamp_ms: u64,
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

const COMPLETED_UNIT_LIMIT: usize = 5;
// Unit IDs are backend-authoritative and must be unique. Keep a bounded recent
// replay guard so an invalid duplicate cannot grow session memory without bound.
const TERMINAL_UNIT_REPLAY_LIMIT: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
struct CaptionUnitKey {
    generation: u64,
    stream_id: String,
    unit_id: String,
}

#[derive(Default)]
struct CaptionSessionState {
    generation_fence: u64,
    snapshot_revision: u64,
    active: Option<CaptionSessionActiveV1>,
    active_units: Vec<CaptionActiveUnitV1>,
    captions: Vec<CaptionSnapshotV1>,
    completed_units: VecDeque<CaptionUnitKey>,
    recent_terminal_units: VecDeque<CaptionUnitKey>,
}

#[derive(Clone, Default)]
pub(crate) struct CaptionSessionStore {
    state: Arc<Mutex<CaptionSessionState>>,
}

impl CaptionSessionStore {
    pub(crate) fn begin_generation(&self, generation: u64) -> AppResult<CaptionSessionSnapshotV1> {
        let mut state = self.lock()?;
        if generation <= state.generation_fence {
            return Ok(Self::snapshot_from(&state));
        }

        state.generation_fence = generation;
        state.active = Some(CaptionSessionActiveV1 {
            generation,
            stream_id: format!("recognition-{generation}-1"),
        });
        state.active_units.clear();
        state.recent_terminal_units.clear();
        state
            .captions
            .retain(|caption| caption.state == CaptionState::Completed);
        Self::advance_revision(&mut state);

        Ok(Self::snapshot_from(&state))
    }

    pub(crate) fn start_unit(
        &self,
        generation: u64,
        stream_id: &str,
        unit_id: String,
        started_at_ms: u64,
    ) -> AppResult<Option<CaptionSessionSnapshotV1>> {
        let mut state = self.lock()?;
        let unit_key = CaptionUnitKey {
            generation,
            stream_id: stream_id.to_string(),
            unit_id: unit_id.clone(),
        };
        if !Self::matches_active(&state, generation, stream_id)
            || state.recent_terminal_units.contains(&unit_key)
            || state
                .active_units
                .iter()
                .any(|unit| unit.unit_id == unit_id)
            || state.completed_units.contains(&unit_key)
        {
            return Ok(None);
        }

        state.active_units.push(CaptionActiveUnitV1 {
            unit_id,
            started_at_ms,
        });
        Self::advance_revision(&mut state);

        Ok(Some(Self::snapshot_from(&state)))
    }

    pub(crate) fn accept_caption(
        &self,
        caption: CaptionSnapshotV1,
    ) -> AppResult<Option<CaptionSessionSnapshotV1>> {
        let mut state = self.lock()?;
        if !Self::matches_active(&state, caption.generation, &caption.stream_id)
            || caption.revision == 0
            || (caption.state == CaptionState::Completed && caption.unit_id.is_none())
        {
            return Ok(None);
        }

        let unit_key = caption.unit_id.as_ref().map(|unit_id| CaptionUnitKey {
            generation: caption.generation,
            stream_id: caption.stream_id.clone(),
            unit_id: unit_id.clone(),
        });
        if unit_key
            .as_ref()
            .is_some_and(|unit_key| state.recent_terminal_units.contains(unit_key))
        {
            return Ok(None);
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
            return Ok(None);
        }

        if let Some(unit_id) = caption.unit_id.as_deref()
            && !state
                .active_units
                .iter()
                .any(|unit| unit.unit_id == unit_id)
        {
            return Ok(None);
        }

        if let Some(index) = existing {
            state.captions.remove(index);
        }

        if caption.state == CaptionState::Completed {
            let Some(unit_id) = caption.unit_id.clone() else {
                return Ok(None);
            };
            state.active_units.retain(|unit| unit.unit_id != unit_id);
            state.captions.insert(0, caption.clone());
            let completed_unit = CaptionUnitKey {
                generation: caption.generation,
                stream_id: caption.stream_id.clone(),
                unit_id,
            };
            Self::record_terminal_unit(&mut state, completed_unit.clone());
            state
                .completed_units
                .retain(|current| current != &completed_unit);
            state.completed_units.push_front(completed_unit);
            Self::trim_completed_units(&mut state);
        } else {
            state.captions.insert(0, caption);
        }
        Self::advance_revision(&mut state);

        Ok(Some(Self::snapshot_from(&state)))
    }

    pub(crate) fn end_unit_without_caption(
        &self,
        generation: u64,
        stream_id: &str,
        unit_id: &str,
    ) -> AppResult<Option<CaptionSessionSnapshotV1>> {
        let mut state = self.lock()?;
        if !Self::matches_active(&state, generation, stream_id)
            || !state
                .active_units
                .iter()
                .any(|unit| unit.unit_id == unit_id)
        {
            return Ok(None);
        }

        state.active_units.retain(|unit| unit.unit_id != unit_id);
        state.captions.retain(|caption| {
            caption.state == CaptionState::Completed
                || caption.generation != generation
                || caption.stream_id != stream_id
                || caption.unit_id.as_deref() != Some(unit_id)
        });
        Self::record_terminal_unit(
            &mut state,
            CaptionUnitKey {
                generation,
                stream_id: stream_id.to_string(),
                unit_id: unit_id.to_string(),
            },
        );
        Self::advance_revision(&mut state);

        Ok(Some(Self::snapshot_from(&state)))
    }

    pub(crate) fn close_generation(
        &self,
        generation: u64,
    ) -> AppResult<Option<CaptionSessionSnapshotV1>> {
        let mut state = self.lock()?;
        if state.active.as_ref().map(|active| active.generation) != Some(generation) {
            return Ok(None);
        }

        state.active = None;
        state.active_units.clear();
        state
            .captions
            .retain(|caption| caption.state == CaptionState::Completed);
        Self::advance_revision(&mut state);

        Ok(Some(Self::snapshot_from(&state)))
    }

    pub(crate) fn snapshot(&self) -> AppResult<CaptionSessionSnapshotV1> {
        let state = self.lock()?;
        Ok(Self::snapshot_from(&state))
    }

    fn lock(&self) -> AppResult<std::sync::MutexGuard<'_, CaptionSessionState>> {
        self.state
            .lock()
            .map_err(|_| AppError::state("Caption session state lock was poisoned."))
    }

    fn matches_active(state: &CaptionSessionState, generation: u64, stream_id: &str) -> bool {
        state
            .active
            .as_ref()
            .is_some_and(|active| active.generation == generation && active.stream_id == stream_id)
    }

    fn advance_revision(state: &mut CaptionSessionState) {
        state.snapshot_revision = state.snapshot_revision.saturating_add(1);
    }

    fn snapshot_from(state: &CaptionSessionState) -> CaptionSessionSnapshotV1 {
        CaptionSessionSnapshotV1 {
            contract_version: CAPTION_SESSION_CONTRACT_VERSION,
            snapshot_revision: state.snapshot_revision,
            active: state.active.clone(),
            active_units: state.active_units.clone(),
            captions: state.captions.clone(),
        }
    }

    fn trim_completed_units(state: &mut CaptionSessionState) {
        while state.completed_units.len() > COMPLETED_UNIT_LIMIT {
            if let Some(expired) = state.completed_units.pop_back() {
                state.captions.retain(|caption| {
                    caption.generation != expired.generation
                        || caption.stream_id != expired.stream_id
                        || caption.unit_id.as_deref() != Some(expired.unit_id.as_str())
                });
            }
        }
    }

    fn record_terminal_unit(state: &mut CaptionSessionState, unit: CaptionUnitKey) {
        state
            .recent_terminal_units
            .retain(|current| current != &unit);
        state.recent_terminal_units.push_back(unit);
        while state.recent_terminal_units.len() > TERMINAL_UNIT_REPLAY_LIMIT {
            state.recent_terminal_units.pop_front();
        }
    }
}

#[cfg(test)]
#[path = "caption_session_tests.rs"]
mod tests;
