//! Bounded OpenAI recognition session.

use crate::caption_session::{CaptionLane, CaptionSnapshotV1, CaptionState};
use crate::config::SttConfig;
use crate::error::AppResult;
use crate::stt::transcribe_openai_wav;
use reqwest::blocking::Client;
use secrecy::SecretString;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) struct CompletedAudioUnit {
    pub(crate) unit_id: String,
    pub(crate) started_at_ms: u64,
    pub(crate) sample_rate_hz: u32,
    pub(crate) samples: Vec<f32>,
}

pub(crate) enum OpenAiBoundedOutcome {
    NoSpeech,
    Completed(CaptionSnapshotV1),
}

type BoundedTranscriber = dyn Fn(&SttConfig, u32, &[f32]) -> AppResult<String> + Send + 'static;

pub(crate) struct OpenAiBoundedSession {
    generation: u64,
    stream_id: String,
    config: SttConfig,
    transcribe: Box<BoundedTranscriber>,
}

impl OpenAiBoundedSession {
    pub(crate) fn new(
        generation: u64,
        stream_id: String,
        config: SttConfig,
        client: Client,
        api_key: SecretString,
    ) -> Self {
        Self::with_transcriber(
            generation,
            stream_id,
            config,
            move |config, sample_rate_hz, samples| {
                transcribe_openai_wav(&client, config, &api_key, sample_rate_hz, samples)
            },
        )
    }

    pub(crate) fn with_transcriber(
        generation: u64,
        stream_id: String,
        config: SttConfig,
        transcribe: impl Fn(&SttConfig, u32, &[f32]) -> AppResult<String> + Send + 'static,
    ) -> Self {
        Self {
            generation,
            stream_id,
            config,
            transcribe: Box::new(transcribe),
        }
    }

    pub(crate) fn recognize(&self, unit: &CompletedAudioUnit) -> AppResult<OpenAiBoundedOutcome> {
        let text = (self.transcribe)(&self.config, unit.sample_rate_hz, &unit.samples)?;
        let text = text.trim();
        if text.is_empty() {
            return Ok(OpenAiBoundedOutcome::NoSpeech);
        }

        Ok(OpenAiBoundedOutcome::Completed(CaptionSnapshotV1 {
            generation: self.generation,
            stream_id: self.stream_id.clone(),
            unit_id: Some(unit.unit_id.clone()),
            lane: CaptionLane::Source,
            revision: 1,
            text: text.to_string(),
            state: CaptionState::Completed,
            language: Some(self.config.language.clone()),
            provider: "openai".to_string(),
            model: self.config.model.clone(),
            unit_started_at_ms: Some(unit.started_at_ms),
            timestamp_ms: now_ms(),
        }))
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

#[cfg(test)]
#[path = "openai_bounded_tests.rs"]
mod tests;
