//! Runtime lifecycle for outgoing captions.
//!
//! The runtime owns one microphone, one active Recognition Module, and one
//! publication policy per generation. Runtime forwards continuous microphone
//! audio and consumes normalized recognition signals; provider adapters own
//! speech units, connection attempts, and protocol I/O.
//!
//! Every utterance announced with `utterance-started` resolves with either a
//! completed caption in the caption-session aggregate or an `utterance-ended`
//! event, so the UI never waits on recognition that cannot arrive. Listening
//! indicators remain distinct lifecycle events rather than placeholder text.
//!
//! Stop is a hard cutoff: the microphone is released within one receive timeout,
//! buffered and queued speech is discarded instead of drained, and no App or
//! Chatbox caption text is committed after the stop request. A state-clearing
//! typing-off packet is sent before waiting for an STT request that is already
//! in flight, so runtime commands must run off the main thread
//! (`#[tauri::command(async)]`) to keep the window responsive during that wait.

mod coordinator;
mod manager;
mod output;
mod supervisor;
#[cfg(test)]
mod test_support;

#[expect(
    unused_imports,
    reason = "AudioProbeLease is part of the curated runtime facade."
)]
pub(crate) use manager::AudioProbeLease;
pub(crate) use manager::{RuntimeManager, RuntimeStartOutcome, RuntimeStartRequest};
pub(crate) use output::{ChatboxPublisherBoundary, RuntimeGeneration};
