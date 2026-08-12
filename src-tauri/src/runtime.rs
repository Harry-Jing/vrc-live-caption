//! Runtime lifecycle for outgoing captions.
//!
//! The runtime owns one microphone, one active Recognition Module, and one
//! publication policy per generation. Runtime forwards continuous microphone
//! audio and consumes normalized recognition signals; recognition drivers own
//! speech units, connection attempts, and protocol I/O.
//!
//! Stop is a hard cutoff: the microphone is released within one receive timeout,
//! buffered and queued speech is discarded instead of drained, and no App or
//! Chatbox caption text is committed after the stop request. A state-clearing
//! typing-off packet is sent before generation-owned workers are joined, so
//! runtime commands must run off the main thread
//! (`#[tauri::command(async)]`) to keep the window responsive during that wait.

mod coordinator;
mod manager;
mod output;
mod supervisor;
#[cfg(test)]
mod test_support;
mod translation;

#[expect(
    unused_imports,
    reason = "AudioProbeLease is part of the curated runtime facade."
)]
pub(crate) use manager::AudioProbeLease;
pub(crate) use manager::{
    PreparedRecognition, RuntimeManager, RuntimeStartOutcome, RuntimeStartRequest,
};
pub(crate) use output::RuntimeGeneration;
pub(crate) use translation::PreparedTranslation;
