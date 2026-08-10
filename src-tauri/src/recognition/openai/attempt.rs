//! OpenAI-private lifecycle for one connected protocol attempt.

use super::super::RecognitionEvent;
use crate::error::AppResult;

#[derive(Clone, Copy, Debug)]
pub(super) struct RecognitionAttemptAudioChunk<'audio> {
    pub(super) sample_rate_hz: u32,
    pub(super) samples: &'audio [f32],
}

/// This seam is intentionally narrower than the active Recognition Interface:
/// runtime and future local drivers never see these unit/commit-shaped calls.
/// `drain_events` is pull-based so the OpenAI Network Owner and deterministic
/// protocol tests share the same state machine. Calling `stop` is a hard output
/// fence; no later provider message may escape as a normalized event.
pub(super) trait RecognitionAttemptSession: Send {
    fn start_unit(&mut self, unit_id: String, started_at_ms: u64) -> AppResult<RecognitionEvent>;

    fn append_audio(&mut self, audio: RecognitionAttemptAudioChunk<'_>) -> AppResult<()>;

    /// Commits the current application-owned OpenAI input unit.
    fn end_input(&mut self) -> AppResult<()>;

    fn drain_events(&mut self, received_at_ms: u64) -> AppResult<Vec<RecognitionEvent>>;

    fn stop(&mut self) -> AppResult<()>;
}
