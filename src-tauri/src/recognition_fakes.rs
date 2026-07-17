//! Deterministic normalized recognition adapters for runtime contract tests
//! and the developer-facing Mock provider.

use crate::caption_session::{CaptionLane, CaptionSnapshotV1, CaptionState};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RecognitionEvent {
    UnitStarted {
        generation: u64,
        stream_id: String,
        unit_id: String,
        started_at_ms: u64,
    },
    Caption(CaptionSnapshotV1),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScriptedRecognitionContext {
    pub(crate) generation: u64,
    pub(crate) stream_id: String,
    pub(crate) language: Option<String>,
    pub(crate) provider: String,
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

pub(crate) struct FakeBoundedRecognitionAdapter {
    context: ScriptedRecognitionContext,
}

impl FakeBoundedRecognitionAdapter {
    pub(crate) fn new(context: ScriptedRecognitionContext) -> Self {
        Self { context }
    }

    pub(crate) fn script_completed(
        &self,
        unit_id: impl Into<String>,
        started_at_ms: u64,
        completed: ScriptedText,
    ) -> Vec<RecognitionEvent> {
        let unit_id = unit_id.into();
        vec![
            unit_started(&self.context, unit_id.clone(), started_at_ms),
            RecognitionEvent::Caption(caption_from_script(
                &self.context,
                Some(unit_id),
                Some(started_at_ms),
                1,
                completed,
                CaptionState::Completed,
            )),
        ]
    }
}

pub(crate) struct FakeOngoingCompletedRecognitionAdapter {
    context: ScriptedRecognitionContext,
}

impl FakeOngoingCompletedRecognitionAdapter {
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
        events.push(unit_started(&self.context, unit_id.clone(), started_at_ms));

        for (index, scripted) in ongoing.iter().cloned().enumerate() {
            events.push(RecognitionEvent::Caption(caption_from_script(
                &self.context,
                Some(unit_id.clone()),
                Some(started_at_ms),
                revision_for_index(index),
                scripted,
                CaptionState::Ongoing,
            )));
        }
        events.push(RecognitionEvent::Caption(caption_from_script(
            &self.context,
            Some(unit_id),
            Some(started_at_ms),
            revision_for_index(ongoing.len()),
            completed,
            CaptionState::Completed,
        )));

        events
    }
}

pub(crate) struct FakeOngoingOnlyRecognitionAdapter {
    context: ScriptedRecognitionContext,
}

impl FakeOngoingOnlyRecognitionAdapter {
    pub(crate) fn new(context: ScriptedRecognitionContext) -> Self {
        Self { context }
    }

    #[cfg(test)]
    pub(crate) fn script_stream(&self, snapshots: &[ScriptedText]) -> Vec<RecognitionEvent> {
        self.script_stream_from(1, snapshots)
    }

    pub(crate) fn script_stream_from(
        &self,
        first_revision: u64,
        snapshots: &[ScriptedText],
    ) -> Vec<RecognitionEvent> {
        snapshots
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, scripted)| {
                RecognitionEvent::Caption(caption_from_script(
                    &self.context,
                    None,
                    None,
                    first_revision.saturating_add(index_as_u64(index)),
                    scripted,
                    CaptionState::Ongoing,
                ))
            })
            .collect()
    }
}

fn unit_started(
    context: &ScriptedRecognitionContext,
    unit_id: String,
    started_at_ms: u64,
) -> RecognitionEvent {
    RecognitionEvent::UnitStarted {
        generation: context.generation,
        stream_id: context.stream_id.clone(),
        unit_id,
        started_at_ms,
    }
}

fn caption_from_script(
    context: &ScriptedRecognitionContext,
    unit_id: Option<String>,
    unit_started_at_ms: Option<u64>,
    revision: u64,
    scripted: ScriptedText,
    state: CaptionState,
) -> CaptionSnapshotV1 {
    CaptionSnapshotV1 {
        generation: context.generation,
        stream_id: context.stream_id.clone(),
        unit_id,
        lane: CaptionLane::Source,
        revision,
        text: scripted.text,
        state,
        language: context.language.clone(),
        provider: context.provider.clone(),
        model: context.model.clone(),
        unit_started_at_ms,
        timestamp_ms: scripted.timestamp_ms,
    }
}

fn revision_for_index(index: usize) -> u64 {
    index_as_u64(index).saturating_add(1)
}

fn index_as_u64(index: usize) -> u64 {
    u64::try_from(index).unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "recognition_fakes_tests.rs"]
mod tests;
