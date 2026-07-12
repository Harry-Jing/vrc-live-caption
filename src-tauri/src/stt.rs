//! Cloud STT transcription for completed speech segments.
//!
//! Captured mono samples are encoded in memory as 16-bit PCM WAV and uploaded
//! to the OpenAI transcriptions endpoint as one blocking request per segment.
//! Blocking is intentional: the dedicated STT worker thread owns the upload,
//! so the capture loop never waits on the network. Callers build one HTTP
//! client per runtime with `build_stt_client` and reuse it across segments to
//! keep connection pooling.

use crate::config::SttConfig;
use crate::error::{AppError, AppResult};
use reqwest::blocking::Client;
use reqwest::blocking::multipart::{Form, Part};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use std::io::Cursor;
use std::time::Duration;

const OPENAI_TRANSCRIPTIONS_URL: &str = "https://api.openai.com/v1/audio/transcriptions";
const STT_REQUEST_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Deserialize)]
struct OpenAiTranscriptionResponse {
    text: String,
}

pub(crate) fn build_stt_client() -> AppResult<Client> {
    Client::builder()
        .timeout(STT_REQUEST_TIMEOUT)
        .build()
        .map_err(|error| AppError::stt(format!("Failed to create STT client: {error}")))
}

pub(crate) fn transcribe_openai_wav(
    client: &Client,
    config: &SttConfig,
    api_key: &SecretString,
    sample_rate: u32,
    samples: &[f32],
) -> AppResult<String> {
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
    let response = client
        .post(OPENAI_TRANSCRIPTIONS_URL)
        .bearer_auth(api_key.expose_secret())
        .multipart(form)
        .send()
        .map_err(|error| map_stt_request_error("STT request failed", error))?;
    let status = response.status();
    let body = response
        .text()
        .map_err(|error| map_stt_request_error("Failed to read STT response", error))?;

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

fn map_stt_request_error(context: &str, error: reqwest::Error) -> AppError {
    if error.is_connect() || error.is_timeout() {
        return AppError::stt_network(format!(
            "Could not reach OpenAI. Check your network connection or system proxy settings. {context}: {error}"
        ));
    }

    AppError::stt(format!("{context}: {error}"))
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
    use std::net::TcpListener;
    use std::thread;

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

    #[test]
    fn timeout_errors_are_reported_as_network_unreachable() -> AppResult<()> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|error| AppError::stt(format!("Failed to bind test server: {error}")))?;
        let address = listener
            .local_addr()
            .map_err(|error| AppError::stt(format!("Failed to read test address: {error}")))?;
        let server = thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                thread::sleep(Duration::from_millis(150));
                drop(stream);
            }
        });
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(30))
            .build()
            .map_err(|error| AppError::stt(format!("Failed to build test client: {error}")))?;
        let request_error = client
            .get(format!("http://{address}"))
            .send()
            .err()
            .ok_or_else(|| AppError::stt("Test request unexpectedly succeeded."))?;
        let error = map_stt_request_error("STT request failed", request_error);

        server
            .join()
            .map_err(|_| AppError::runtime("Network timeout test server panicked."))?;

        assert_eq!(error.code(), "stt.network_unreachable");
        assert!(error.to_string().contains("network connection"));
        assert!(error.to_string().contains("system proxy"));

        Ok(())
    }
}
