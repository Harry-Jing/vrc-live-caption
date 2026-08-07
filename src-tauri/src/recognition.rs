//! Provider-neutral recognition-session boundary.
//!
//! Provider adapters own wire deltas and expose only full normalized caption
//! snapshots. Runtime depends on this seam so the Realtime protocol state
//! machine remains testable without a microphone, socket, Tauri handle, or
//! publication policy.

use crate::caption_session::CaptionSnapshotV1;
use crate::error::AppResult;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RecognitionEvent {
    UnitStarted {
        generation: u64,
        stream_id: String,
        unit_id: String,
        started_at_ms: u64,
    },
    UnitEnded {
        generation: u64,
        stream_id: String,
        unit_id: String,
        reason: RecognitionEndReason,
    },
    Caption(CaptionSnapshotV1),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RecognitionEndReason {
    NoSpeech,
    Failed { detail: String },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RecognitionAudioChunk<'audio> {
    pub(crate) sample_rate_hz: u32,
    pub(crate) samples: &'audio [f32],
}

/// The app-facing lifecycle of one selected recognition model.
///
/// `drain_events` is deliberately pull-based: a transport driver can call it
/// on its worker thread, while deterministic tests can inject server events.
/// Calling `stop` is a hard output fence; no later provider message may escape
/// as a normalized event.
pub(crate) trait RecognitionSession: Send {
    fn start_unit(&mut self, unit_id: String, started_at_ms: u64) -> AppResult<RecognitionEvent>;

    fn append_audio(&mut self, audio: RecognitionAudioChunk<'_>) -> AppResult<()>;

    /// Ends the current provider-independent input unit. An adapter may map
    /// this to an OpenAI buffer commit, a local worker message, or another
    /// implementation-specific boundary.
    fn end_input(&mut self) -> AppResult<()>;

    fn drain_events(&mut self, received_at_ms: u64) -> AppResult<Vec<RecognitionEvent>>;

    fn stop(&mut self) -> AppResult<()>;
}
