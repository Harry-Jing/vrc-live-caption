//! Deterministic normalized recognition events for runtime contract tests.

use crate::caption_session::{CaptionLane, CaptionSnapshotV1, CaptionState};
use crate::recognition::{RecognitionEndReason, RecognitionEvent};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScriptedRecognitionContext {
    pub(crate) generation: u64,
    pub(crate) stream_id: String,
    pub(crate) language: Option<String>,
    pub(crate) model: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScriptedText {
    text: String,
    timestamp_ms: u64,
}

impl ScriptedText {
    pub(crate) fn new(text: impl Into<String>, timestamp_ms: u64) -> Self {
        Self {
            text: text.into(),
            timestamp_ms,
        }
    }
}

pub(crate) struct ScriptedRecognitionAdapter {
    context: ScriptedRecognitionContext,
}

impl ScriptedRecognitionAdapter {
    pub(crate) fn new(context: ScriptedRecognitionContext) -> Self {
        Self { context }
    }

    pub(crate) fn script_unit(
        &self,
        unit_id: impl Into<String>,
        started_at_ms: u64,
        ongoing: &[ScriptedText],
        completed: ScriptedText,
    ) -> Vec<RecognitionEvent> {
        let unit_id = unit_id.into();
        let mut events = Vec::with_capacity(ongoing.len().saturating_add(2));
        events.push(self.unit_started(unit_id.clone(), started_at_ms));

        for (index, scripted) in ongoing.iter().cloned().enumerate() {
            events.push(RecognitionEvent::Caption(self.caption(
                unit_id.clone(),
                started_at_ms,
                revision_for_index(index),
                scripted,
                CaptionState::Ongoing,
            )));
        }
        events.push(RecognitionEvent::Caption(self.caption(
            unit_id,
            started_at_ms,
            revision_for_index(ongoing.len()),
            completed,
            CaptionState::Completed,
        )));

        events
    }

    pub(crate) fn script_ended(
        &self,
        unit_id: impl Into<String>,
        started_at_ms: u64,
        reason: RecognitionEndReason,
    ) -> Vec<RecognitionEvent> {
        let unit_id = unit_id.into();
        vec![
            self.unit_started(unit_id.clone(), started_at_ms),
            RecognitionEvent::UnitEnded {
                generation: self.context.generation,
                stream_id: self.context.stream_id.clone(),
                unit_id,
                reason,
            },
        ]
    }

    fn unit_started(&self, unit_id: String, started_at_ms: u64) -> RecognitionEvent {
        RecognitionEvent::UnitStarted {
            generation: self.context.generation,
            stream_id: self.context.stream_id.clone(),
            unit_id,
            started_at_ms,
        }
    }

    fn caption(
        &self,
        unit_id: String,
        started_at_ms: u64,
        revision: u64,
        scripted: ScriptedText,
        state: CaptionState,
    ) -> CaptionSnapshotV1 {
        CaptionSnapshotV1 {
            generation: self.context.generation,
            stream_id: self.context.stream_id.clone(),
            unit_id: Some(unit_id),
            lane: CaptionLane::Source,
            revision,
            text: scripted.text,
            state,
            language: self.context.language.clone(),
            provider: "openai".to_string(),
            model: self.context.model.clone(),
            unit_started_at_ms: Some(started_at_ms),
            timestamp_ms: scripted.timestamp_ms,
        }
    }
}

fn revision_for_index(index: usize) -> u64 {
    u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1)
}

#[cfg(test)]
#[path = "fakes_tests.rs"]
mod tests;
