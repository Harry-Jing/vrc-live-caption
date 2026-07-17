use super::*;
use crate::caption_session::{CaptionLane, CaptionState};
use crate::config::AppConfig;
use crate::error::AppResult;

#[test]
fn bounded_openai_returns_zero_or_one_completed_source_snapshot() -> AppResult<()> {
    let config = AppConfig::default();
    let completed = OpenAiBoundedSession::with_transcriber(
        7,
        "recognition-7-1".to_string(),
        config.stt.clone(),
        |_config, _sample_rate_hz, _samples| Ok("recognized speech".to_string()),
    );
    let unit = CompletedAudioUnit {
        unit_id: "speech-7-1".to_string(),
        started_at_ms: 1_000,
        sample_rate_hz: 16_000,
        samples: vec![0.0; 160],
    };

    let OpenAiBoundedOutcome::Completed(snapshot) = completed.recognize(&unit)? else {
        return Err(crate::error::AppError::stt(
            "Test recognition unexpectedly returned no speech.",
        ));
    };
    assert_eq!(snapshot.generation, 7);
    assert_eq!(snapshot.stream_id, "recognition-7-1");
    assert_eq!(snapshot.unit_id.as_deref(), Some("speech-7-1"));
    assert_eq!(snapshot.lane, CaptionLane::Source);
    assert_eq!(snapshot.revision, 1);
    assert_eq!(snapshot.text, "recognized speech");
    assert_eq!(snapshot.state, CaptionState::Completed);
    assert_eq!(snapshot.language.as_deref(), Some("en"));
    assert_eq!(snapshot.provider, "openai");
    assert_eq!(snapshot.model, "gpt-4o-mini-transcribe");
    assert_eq!(snapshot.unit_started_at_ms, Some(1_000));

    let empty = OpenAiBoundedSession::with_transcriber(
        7,
        "recognition-7-1".to_string(),
        config.stt,
        |_config, _sample_rate_hz, _samples| Ok(String::new()),
    );
    assert!(matches!(
        empty.recognize(&unit)?,
        OpenAiBoundedOutcome::NoSpeech
    ));

    Ok(())
}
