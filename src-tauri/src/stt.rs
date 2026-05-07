use crate::config::SttConfig;
use crate::error::{AppError, AppResult};
use reqwest::blocking::multipart::{Form, Part};
use serde::Deserialize;
use std::env;
use std::io::Cursor;
use std::time::Duration;

const OPENAI_TRANSCRIPTIONS_URL: &str = "https://api.openai.com/v1/audio/transcriptions";
const OPENAI_API_KEY_ENV: &str = "OPENAI_API_KEY";

#[derive(Deserialize)]
struct OpenAiTranscriptionResponse {
    text: String,
}

pub(crate) fn transcribe_openai_wav(
    config: &SttConfig,
    sample_rate: u32,
    samples: &[f32],
) -> AppResult<String> {
    let api_key = openai_api_key()?;
    let wav_bytes = encode_wav(sample_rate, samples)?;
    let part = Part::bytes(wav_bytes)
        .file_name("speech.wav")
        .mime_str("audio/wav")
        .map_err(|error| AppError::stt(format!("Failed to build STT upload: {error}")))?;
    let form = Form::new()
        .text("model", config.model.clone())
        .text("language", normalize_language(&config.language))
        .text("response_format", "json")
        .part("file", part);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|error| AppError::stt(format!("Failed to create STT client: {error}")))?;
    let response = client
        .post(OPENAI_TRANSCRIPTIONS_URL)
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .map_err(|error| AppError::stt(format!("STT request failed: {error}")))?;
    let status = response.status();
    let body = response
        .text()
        .map_err(|error| AppError::stt(format!("Failed to read STT response: {error}")))?;

    if !status.is_success() {
        return Err(AppError::stt(format!(
            "STT provider returned {status}: {}",
            truncate_for_diagnostic(&body)
        )));
    }

    let parsed: OpenAiTranscriptionResponse = serde_json::from_str(&body)
        .map_err(|error| AppError::stt(format!("Failed to parse STT response: {error}")))?;

    Ok(parsed.text.trim().to_string())
}

pub(crate) fn ensure_openai_api_key() -> AppResult<()> {
    openai_api_key().map(|_| ())
}

fn openai_api_key() -> AppResult<String> {
    env::var(OPENAI_API_KEY_ENV).map_err(|_| {
        AppError::stt(format!(
            "Missing {OPENAI_API_KEY_ENV}. Set it in the environment before starting cloud STT."
        ))
    })
}

fn encode_wav(sample_rate: u32, samples: &[f32]) -> AppResult<Vec<u8>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut wav_bytes = Vec::new();
    let cursor = Cursor::new(&mut wav_bytes);
    let mut writer =
        hound::WavWriter::new(cursor, spec).map_err(|error| AppError::wav(error.to_string()))?;

    for sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let pcm = (clamped * f32::from(i16::MAX)).round() as i16;

        writer
            .write_sample(pcm)
            .map_err(|error| AppError::wav(error.to_string()))?;
    }

    writer
        .finalize()
        .map_err(|error| AppError::wav(error.to_string()))?;

    Ok(wav_bytes)
}

fn normalize_language(language: &str) -> String {
    language
        .split(['-', '_'])
        .next()
        .filter(|part| !part.trim().is_empty())
        .unwrap_or("en")
        .to_lowercase()
}

fn truncate_for_diagnostic(message: &str) -> String {
    const MAX_CHARS: usize = 320;

    let mut truncated = String::new();

    for (index, character) in message.chars().enumerate() {
        if index >= MAX_CHARS {
            truncated.push_str("...");
            return truncated;
        }

        truncated.push(character);
    }

    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_is_normalized_for_openai_transcriptions() {
        assert_eq!(normalize_language("en-US"), "en");
        assert_eq!(normalize_language("ja_JP"), "ja");
        assert_eq!(normalize_language(""), "en");
    }

    #[test]
    fn wav_encoder_outputs_non_empty_wav() -> AppResult<()> {
        let wav = encode_wav(16_000, &[0.0, 0.25, -0.25])?;

        assert!(wav.starts_with(b"RIFF"));
        assert!(wav.len() > 44);

        Ok(())
    }
}
