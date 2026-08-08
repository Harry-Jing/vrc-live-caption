use super::*;
use crate::caption_session::{CaptionSnapshotV1, CaptionState};
use crate::error::{AppError, AppResult, ProviderFailureClass, RetryDisposition};
use crate::recognition::{
    RecognitionAudioChunk, RecognitionEndReason, RecognitionEvent, RecognitionSession,
};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Default)]
struct FakeTransportState {
    sent: Vec<String>,
    received: VecDeque<String>,
    close_count: usize,
}

#[derive(Clone, Default)]
struct FakeTransportProbe {
    state: Arc<Mutex<FakeTransportState>>,
}

impl FakeTransportProbe {
    fn lock(&self) -> AppResult<MutexGuard<'_, FakeTransportState>> {
        self.state
            .lock()
            .map_err(|_| AppError::state("Fake Realtime transport lock was poisoned."))
    }

    fn push_server_event(&self, event: Value) -> AppResult<()> {
        self.lock()?.received.push_back(event.to_string());
        Ok(())
    }

    fn push_raw_server_event(&self, event: impl Into<String>) -> AppResult<()> {
        self.lock()?.received.push_back(event.into());
        Ok(())
    }

    fn sent_json(&self) -> AppResult<Vec<Value>> {
        self.lock()?
            .sent
            .iter()
            .map(|message| {
                serde_json::from_str(message).map_err(|error| {
                    AppError::state(format!("Fake transport recorded invalid JSON: {error}"))
                })
            })
            .collect()
    }

    fn close_count(&self) -> AppResult<usize> {
        Ok(self.lock()?.close_count)
    }
}

struct FakeTransport {
    probe: FakeTransportProbe,
}

#[derive(Clone, Default)]
struct TracingCapture {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl TracingCapture {
    fn writer(&self) -> TracingCaptureWriter {
        TracingCaptureWriter {
            bytes: self.bytes.clone(),
        }
    }

    fn contents(&self) -> AppResult<String> {
        let bytes = self
            .bytes
            .lock()
            .map_err(|_| AppError::state("Tracing capture lock was poisoned."))?
            .clone();
        String::from_utf8(bytes)
            .map_err(|error| AppError::state(format!("Tracing output was not UTF-8: {error}")))
    }
}

struct TracingCaptureWriter {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl Write for TracingCaptureWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.bytes
            .lock()
            .map_err(|_| io::Error::other("Tracing capture lock was poisoned."))?
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl FakeTransport {
    fn new() -> (Self, FakeTransportProbe) {
        let probe = FakeTransportProbe::default();
        (
            Self {
                probe: probe.clone(),
            },
            probe,
        )
    }
}

impl RealtimeTransport for FakeTransport {
    fn send_text(&mut self, message: String) -> AppResult<()> {
        self.probe.lock()?.sent.push(message);
        Ok(())
    }

    fn try_receive_text(&mut self) -> AppResult<Option<String>> {
        Ok(self.probe.lock()?.received.pop_front())
    }

    fn close(&mut self) -> AppResult<()> {
        self.probe.lock()?.close_count += 1;
        Ok(())
    }
}

#[derive(Clone, Default)]
struct ManualClock {
    elapsed_ms: Arc<AtomicU64>,
}

impl ManualClock {
    fn advance_ms(&self, elapsed_ms: u64) -> AppResult<()> {
        self.elapsed_ms
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                current.checked_add(elapsed_ms)
            })
            .map(|_| ())
            .map_err(|_| AppError::state("Manual monotonic clock exceeded its supported range."))
    }
}

impl MonotonicClock for ManualClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.elapsed_ms.load(Ordering::SeqCst))
    }
}

fn session(
    model: OpenAiTranscriptionModel,
    languages: &[&str],
) -> AppResult<(OpenAiRealtimeSession<FakeTransport>, FakeTransportProbe)> {
    let (transport, probe) = FakeTransport::new();
    let session = OpenAiRealtimeSession::connect(
        OpenAiRealtimeSessionContext {
            generation: 7,
            connection_epoch: 3,
            stream_id: "recognition-7-1".to_string(),
        },
        model,
        languages.iter().map(|value| (*value).to_string()).collect(),
        transport,
    )?;
    Ok((session, probe))
}

fn session_with_manual_clock(
    model: OpenAiTranscriptionModel,
    languages: &[&str],
) -> AppResult<(
    OpenAiRealtimeSession<FakeTransport>,
    FakeTransportProbe,
    ManualClock,
)> {
    let (transport, probe) = FakeTransport::new();
    let clock = ManualClock::default();
    let session = OpenAiRealtimeSession::connect_with_clock(
        OpenAiRealtimeSessionContext {
            generation: 7,
            connection_epoch: 3,
            stream_id: "recognition-7-1".to_string(),
        },
        model,
        languages.iter().map(|value| (*value).to_string()).collect(),
        transport,
        Box::new(clock.clone()),
    )?;
    Ok((session, probe, clock))
}

fn start_unit(
    session: &mut impl RecognitionSession,
    unit_id: &str,
    started_at_ms: u64,
) -> AppResult<()> {
    let event = session.start_unit(unit_id.to_string(), started_at_ms)?;
    assert!(matches!(
        event,
        RecognitionEvent::UnitStarted {
            generation: 7,
            ref stream_id,
            ref unit_id,
            started_at_ms: actual_started_at_ms,
        } if stream_id == "recognition-7-1"
            && unit_id.starts_with("unit-")
            && actual_started_at_ms == started_at_ms
    ));
    Ok(())
}

fn captions(events: Vec<RecognitionEvent>) -> Vec<CaptionSnapshotV1> {
    events
        .into_iter()
        .filter_map(|event| match event {
            RecognitionEvent::Caption(caption) => Some(caption),
            RecognitionEvent::UnitStarted { .. } | RecognitionEvent::UnitEnded { .. } => None,
        })
        .collect()
}

fn buffer_committed(item_id: &str) -> Value {
    json!({
        "type": "input_audio_buffer.committed",
        "item_id": item_id,
    })
}

fn transcript_delta(item_id: &str, delta: &str) -> Value {
    json!({
        "type": "conversation.item.input_audio_transcription.delta",
        "item_id": item_id,
        "delta": delta,
    })
}

fn transcript_completed(item_id: &str, transcript: &str) -> Value {
    json!({
        "type": "conversation.item.input_audio_transcription.completed",
        "item_id": item_id,
        "transcript": transcript,
    })
}

fn transcript_completed_with_languages(
    item_id: &str,
    transcript: &str,
    languages: &[&str],
) -> Value {
    json!({
        "type": "conversation.item.input_audio_transcription.completed",
        "item_id": item_id,
        "transcript": transcript,
        "languages": languages
            .iter()
            .map(|code| json!({ "code": code }))
            .collect::<Vec<_>>(),
    })
}

fn transcript_failed(item_id: &str, message: &str) -> Value {
    json!({
        "type": "conversation.item.input_audio_transcription.failed",
        "item_id": item_id,
        "error": { "message": message },
    })
}

#[test]
fn release_catalog_accepts_only_exact_transcription_models() -> AppResult<()> {
    assert_eq!(
        OpenAiTranscriptionModel::try_from("gpt-transcribe")?,
        OpenAiTranscriptionModel::GptTranscribe
    );
    assert_eq!(
        OpenAiTranscriptionModel::try_from("gpt-live-transcribe")?,
        OpenAiTranscriptionModel::GptLiveTranscribe
    );
    assert!(OpenAiTranscriptionModel::try_from("gpt-4o-transcribe").is_err());
    assert!(OpenAiTranscriptionModel::try_from("gpt-realtime-whisper").is_err());
    Ok(())
}

#[test]
fn connection_configures_a_transcription_session_with_pcm_24k_and_languages() -> AppResult<()> {
    let (_session, probe) = session(OpenAiTranscriptionModel::GptLiveTranscribe, &["en", "zh"])?;
    let sent = probe.sent_json()?;

    assert_eq!(
        sent,
        vec![json!({
            "event_id": "vrc-session-update-7-3",
            "type": "session.update",
            "session": {
                "type": "transcription",
                "audio": {
                    "input": {
                        "format": {
                            "type": "audio/pcm",
                            "rate": 24_000,
                        },
                        "transcription": {
                            "model": "gpt-live-transcribe",
                            "languages": ["en", "zh"],
                        },
                        "turn_detection": null,
                    }
                }
            }
        })]
    );
    assert!(!sent[0].to_string().contains("\"language\":"));
    Ok(())
}

#[test]
fn session_is_not_ready_until_openai_confirms_the_update() -> AppResult<()> {
    let (mut session, probe) = session(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    assert!(!session.is_ready());

    probe.push_server_event(json!({ "type": "session.updated", "session": {} }))?;
    assert!(session.drain_events(10)?.is_empty());

    assert!(session.is_ready());
    Ok(())
}

#[test]
fn append_encodes_mono_pcm16_at_24k_then_commit_is_a_separate_event() -> AppResult<()> {
    let (mut session, probe) = session(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut session, "unit-a", 100)?;

    session.append_audio(RecognitionAudioChunk {
        sample_rate_hz: 24_000,
        samples: &[0.0, 1.0, -1.0],
    })?;
    session.end_input()?;

    let sent = probe.sent_json()?;
    assert_eq!(sent.len(), 3);
    assert_eq!(
        sent[1],
        json!({
            "type": "input_audio_buffer.append",
            "audio": "AAD/fwGA",
        })
    );
    assert_eq!(
        sent[2],
        json!({
            "event_id": "vrc-commit-7-3-0",
            "type": "input_audio_buffer.commit",
        })
    );
    Ok(())
}

#[test]
fn provider_error_does_not_escape_through_app_error_display_or_serialization() -> AppResult<()> {
    let (mut session, probe) = session(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    let canaries = [
        "provider-message-canary",
        "provider-type-canary",
        "provider-code-canary",
        "provider-param-canary",
        "provider-event-id-canary",
    ];
    probe.push_server_event(json!({
        "type": "error",
        "error": {
            "type": canaries[1],
            "code": canaries[2],
            "message": canaries[0],
            "param": canaries[3],
            "event_id": canaries[4],
        }
    }))?;

    let error = session
        .drain_events(100)
        .err()
        .ok_or_else(|| AppError::state("A provider error unexpectedly succeeded."))?;
    let display = error.to_string();
    let serialized = serde_json::to_string(&error)
        .map_err(|error| AppError::state(format!("Failed to serialize provider error: {error}")))?;

    assert_eq!(error.code(), "stt.provider_failed");
    assert_eq!(
        error.provider_failure_class(),
        Some(ProviderFailureClass::Unknown)
    );
    assert_eq!(error.retry_disposition(), RetryDisposition::Terminal);
    assert_eq!(display, "OpenAI Realtime transcription failed.");
    for canary in canaries {
        assert!(!display.contains(canary));
        assert!(!serialized.contains(canary));
    }
    assert_eq!(probe.close_count()?, 1);
    Ok(())
}

#[test]
fn provider_error_classification_uses_structured_fields_not_message_text() -> AppResult<()> {
    let (mut session, probe) = session(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    probe.push_server_event(json!({
        "type": "error",
        "error": {
            "type": "invalid_request_error",
            "code": "invalid_value",
            "message": "server_error rate_limit_exceeded invalid_api_key",
            "param": "message-text-must-not-control-classification",
            "event_id": "classification-event-canary",
        }
    }))?;

    let error = session
        .drain_events(100)
        .err()
        .ok_or_else(|| AppError::state("A provider error unexpectedly succeeded."))?;

    assert_eq!(
        error.provider_failure_class(),
        Some(ProviderFailureClass::InvalidRequest)
    );
    assert_eq!(error.retry_disposition(), RetryDisposition::Terminal);
    assert_eq!(error.code(), "stt.provider_invalid_request");
    assert_eq!(
        error.to_string(),
        "OpenAI rejected the Realtime transcription request."
    );
    Ok(())
}

#[test]
fn provider_error_classes_have_stable_retry_dispositions() -> AppResult<()> {
    let cases = [
        (
            "authentication_error",
            None,
            ProviderFailureClass::Authentication,
            RetryDisposition::Terminal,
            "stt.provider_authentication_failed",
        ),
        (
            "permission_error",
            None,
            ProviderFailureClass::PermissionDenied,
            RetryDisposition::Terminal,
            "stt.provider_permission_denied",
        ),
        (
            "rate_limit_error",
            None,
            ProviderFailureClass::RateLimited,
            RetryDisposition::Retryable,
            "stt.provider_rate_limited",
        ),
        (
            "insufficient_quota",
            Some("credit_balance_exhausted"),
            ProviderFailureClass::UsageLimit,
            RetryDisposition::Terminal,
            "stt.provider_usage_limit",
        ),
        (
            "server_error",
            None,
            ProviderFailureClass::ServiceUnavailable,
            RetryDisposition::Retryable,
            "stt.provider_unavailable",
        ),
    ];

    for (kind, code, expected_class, expected_retry, expected_code) in cases {
        let (mut session, probe) = session(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
        probe.push_server_event(json!({
            "type": "error",
            "error": {
                "type": kind,
                "code": code,
                "message": "invalid_request_error must not control this classification",
            }
        }))?;

        let error = session
            .drain_events(100)
            .err()
            .ok_or_else(|| AppError::state("A provider error unexpectedly succeeded."))?;
        assert_eq!(error.provider_failure_class(), Some(expected_class));
        assert_eq!(error.retry_disposition(), expected_retry);
        assert_eq!(error.code(), expected_code);
    }
    Ok(())
}

#[test]
fn provider_error_does_not_escape_through_tracing() -> AppResult<()> {
    let (mut session, probe) = session(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    let canaries = [
        "trace-message-canary",
        "trace-type-canary",
        "trace-code-canary",
        "trace-param-canary",
        "trace-event-id-canary",
    ];
    probe.push_server_event(json!({
        "type": "error",
        "error": {
            "type": canaries[1],
            "code": canaries[2],
            "message": canaries[0],
            "param": canaries[3],
            "event_id": canaries[4],
        }
    }))?;

    let capture = TracingCapture::default();
    let writer_capture = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_writer(move || writer_capture.writer())
        .finish();
    let result = tracing::subscriber::with_default(subscriber, || session.drain_events(100));
    assert!(result.is_err());

    let tracing_output = capture.contents()?;
    assert!(tracing_output.contains("OpenAI Realtime provider failure"));
    for canary in canaries {
        assert!(!tracing_output.contains(canary));
    }
    Ok(())
}

#[test]
fn malformed_provider_metadata_is_discarded_without_entering_parser_errors() -> AppResult<()> {
    let (mut session, probe) = session(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    let numeric_canary = 31_415_926_535_u64;
    let nested_canary = "nested-provider-code-canary";
    probe.push_server_event(json!({
        "type": "error",
        "error": {
            "type": numeric_canary,
            "code": { "value": nested_canary },
            "message": ["provider-message-array-canary"],
        }
    }))?;

    let error = session
        .drain_events(100)
        .err()
        .ok_or_else(|| AppError::state("A malformed provider error unexpectedly succeeded."))?;
    let observable = format!("{error:?}\n{error}");

    assert_eq!(
        error.provider_failure_class(),
        Some(ProviderFailureClass::Unknown)
    );
    assert!(!observable.contains(&numeric_canary.to_string()));
    assert!(!observable.contains(nested_canary));
    assert!(!observable.contains("provider-message-array-canary"));
    Ok(())
}

#[test]
fn malformed_provider_error_shape_cannot_escape_through_the_json_decoder() -> AppResult<()> {
    let (mut session, probe) = session(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    let canary = "provider-message-canary";
    probe.push_raw_server_event(format!(r#"{{"type":"error","error":"{canary}"}}"#))?;

    let error = session
        .drain_events(100)
        .err()
        .ok_or_else(|| AppError::state("A malformed provider event unexpectedly succeeded."))?;
    let observable = format!("{error:?}\n{error}");

    assert_eq!(error.code(), "stt.failed");
    assert_eq!(
        error.to_string(),
        "OpenAI Realtime returned an invalid server event."
    );
    assert!(!observable.contains(canary));
    Ok(())
}

#[test]
fn gpt_transcribe_suppresses_deltas_and_emits_only_the_completed_snapshot() -> AppResult<()> {
    let (mut session, probe) = session(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut session, "unit-a", 100)?;
    session.end_input()?;

    probe.push_server_event(buffer_committed("item-a"))?;
    probe.push_server_event(transcript_delta("item-a", "not downstream"))?;
    probe.push_server_event(transcript_completed("item-a", "final transcript"))?;

    let captions = captions(session.drain_events(180)?);
    assert_eq!(captions.len(), 1);
    assert_eq!(captions[0].unit_id.as_deref(), Some("unit-a"));
    assert_eq!(captions[0].text, "final transcript");
    assert_eq!(captions[0].revision, 1);
    assert_eq!(captions[0].state, CaptionState::Completed);
    assert_eq!(captions[0].language, None);
    assert_eq!(captions[0].provider, "openai");
    assert_eq!(captions[0].model, "gpt-transcribe");
    assert_eq!(captions[0].unit_started_at_ms, Some(100));
    assert_eq!(captions[0].timestamp_ms, 180);
    Ok(())
}

#[test]
fn gpt_live_transcribe_emits_full_ongoing_snapshots_before_commit_and_a_final() -> AppResult<()> {
    let (mut session, probe) = session(OpenAiTranscriptionModel::GptLiveTranscribe, &["en"])?;
    start_unit(&mut session, "unit-live", 200)?;

    probe.push_server_event(transcript_delta("item-live", "hello"))?;
    probe.push_server_event(transcript_delta("item-live", " world"))?;
    let ongoing = captions(session.drain_events(240)?);
    assert_eq!(
        ongoing
            .iter()
            .map(|caption| (caption.text.as_str(), caption.revision, caption.state))
            .collect::<Vec<_>>(),
        vec![
            ("hello", 1, CaptionState::Ongoing),
            ("hello world", 2, CaptionState::Ongoing),
        ]
    );

    session.end_input()?;
    probe.push_server_event(buffer_committed("item-live"))?;
    probe.push_server_event(transcript_completed("item-live", "Hello world."))?;
    let completed = captions(session.drain_events(300)?);
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].text, "Hello world.");
    assert_eq!(completed[0].revision, 3);
    assert_eq!(completed[0].state, CaptionState::Completed);
    assert_eq!(completed[0].model, "gpt-live-transcribe");
    assert_eq!(completed[0].language, None);
    Ok(())
}

#[test]
fn live_item_binding_replays_earlier_delta_before_the_current_delta() -> AppResult<()> {
    let (mut session, probe) = session(OpenAiTranscriptionModel::GptLiveTranscribe, &["en"])?;
    start_unit(&mut session, "unit-a", 100)?;
    session.end_input()?;
    start_unit(&mut session, "unit-b", 120)?;

    // Unit A is committed but not yet bound, so B's first early delta cannot
    // be attached safely and must wait for A's provider item binding.
    probe.push_server_event(transcript_delta("item-b", "hello"))?;
    assert!(session.drain_events(130)?.is_empty());

    probe.push_server_event(buffer_committed("item-a"))?;
    probe.push_server_event(transcript_delta("item-b", " world"))?;
    let captions = captions(session.drain_events(140)?);

    assert_eq!(captions.len(), 2);
    assert_eq!(captions[0].unit_id.as_deref(), Some("unit-b"));
    assert_eq!(captions[0].revision, 1);
    assert_eq!(captions[0].text, "hello");
    assert_eq!(captions[1].revision, 2);
    assert_eq!(captions[1].text, "hello world");
    Ok(())
}

#[test]
fn gpt_transcribe_uses_provider_detection_instead_of_language_hints() -> AppResult<()> {
    let (mut session, probe) = session(OpenAiTranscriptionModel::GptTranscribe, &["en", "fr"])?;
    start_unit(&mut session, "unit-a", 100)?;
    session.end_input()?;
    probe.push_server_event(buffer_committed("item-a"))?;
    probe.push_server_event(transcript_completed_with_languages(
        "item-a",
        "Bonjour.",
        &["fr"],
    ))?;

    let captions = captions(session.drain_events(160)?);
    assert_eq!(captions.len(), 1);
    assert_eq!(captions[0].language.as_deref(), Some("fr"));
    Ok(())
}

#[test]
fn multiple_detected_languages_are_not_collapsed_into_a_singular_label() -> AppResult<()> {
    let (mut session, probe) = session(OpenAiTranscriptionModel::GptTranscribe, &["zh", "en"])?;
    start_unit(&mut session, "unit-a", 100)?;
    session.end_input()?;
    probe.push_server_event(buffer_committed("item-a"))?;
    probe.push_server_event(transcript_completed_with_languages(
        "item-a",
        "你好, world.",
        &["zh", "en"],
    ))?;

    let captions = captions(session.drain_events(160)?);
    assert_eq!(captions.len(), 1);
    assert_eq!(captions[0].language, None);
    Ok(())
}

#[test]
fn outstanding_turns_are_bounded_without_dropping_an_existing_turn() -> AppResult<()> {
    let (mut session, _probe) = session(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;

    for index in 0..MAX_OUTSTANDING_TURNS {
        start_unit(&mut session, &format!("unit-{index}"), index as u64)?;
        session.end_input()?;
    }

    let error = session
        .start_unit("unit-overflow".to_string(), 999)
        .err()
        .ok_or_else(|| AppError::state("An unbounded recognition turn unexpectedly started."))?;
    assert!(error.to_string().contains("outstanding recognition units"));
    Ok(())
}

#[test]
fn uncorrelated_provider_items_and_events_are_bounded() -> AppResult<()> {
    let (mut item_session, item_probe) = session(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;

    for index in 0..MAX_PENDING_PROVIDER_ITEMS {
        item_probe.push_server_event(transcript_completed(&format!("item-{index}"), "pending"))?;
    }
    item_probe.push_server_event(transcript_completed("item-overflow", "pending"))?;
    let item_error = item_session
        .drain_events(100)
        .err()
        .ok_or_else(|| AppError::state("Unbounded provider items were unexpectedly accepted."))?;
    assert!(
        item_error
            .to_string()
            .contains("uncorrelated provider items")
    );

    let (mut event_session, event_probe) =
        session(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    for _ in 0..=MAX_PENDING_EVENTS_PER_ITEM {
        event_probe.push_server_event(transcript_completed("item-a", "pending"))?;
    }
    assert!(event_session.drain_events(120)?.is_empty());
    let event_error = event_session
        .drain_events(121)
        .err()
        .ok_or_else(|| AppError::state("Unbounded provider events were unexpectedly accepted."))?;
    assert!(
        event_error
            .to_string()
            .contains("pending events for one item")
    );
    Ok(())
}

#[test]
fn provider_transcript_text_is_bounded_per_turn() -> AppResult<()> {
    let (mut session, probe) = session(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut session, "unit-a", 100)?;
    session.end_input()?;
    probe.push_server_event(buffer_committed("item-a"))?;
    probe.push_server_event(transcript_completed(
        "item-a",
        &"x".repeat(MAX_TRANSCRIPT_BYTES_PER_TURN + 1),
    ))?;

    let error = session
        .drain_events(160)
        .err()
        .ok_or_else(|| AppError::state("An oversized transcript unexpectedly succeeded."))?;
    assert!(error.to_string().contains("per-turn text limit"));
    Ok(())
}

#[test]
fn completed_items_are_released_in_local_turn_order_using_item_ids() -> AppResult<()> {
    let (mut session, probe) = session(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut session, "unit-a", 100)?;
    session.end_input()?;
    start_unit(&mut session, "unit-b", 200)?;
    session.end_input()?;

    probe.push_server_event(buffer_committed("item-a"))?;
    probe.push_server_event(buffer_committed("item-b"))?;
    assert!(session.drain_events(220)?.is_empty());

    probe.push_server_event(transcript_completed("item-b", "second"))?;
    assert!(session.drain_events(260)?.is_empty());

    probe.push_server_event(transcript_completed("item-a", "first"))?;
    let completed = captions(session.drain_events(280)?);
    assert_eq!(
        completed
            .iter()
            .map(|caption| (caption.unit_id.as_deref(), caption.text.as_str()))
            .collect::<Vec<_>>(),
        vec![(Some("unit-a"), "first"), (Some("unit-b"), "second")]
    );
    Ok(())
}

#[test]
fn completion_can_arrive_before_its_item_binding() -> AppResult<()> {
    let (mut session, probe) = session(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut session, "unit-a", 100)?;
    session.end_input()?;

    probe.push_server_event(transcript_completed("item-a", "final"))?;
    assert!(session.drain_events(140)?.is_empty());
    probe.push_server_event(buffer_committed("item-a"))?;
    let completed = captions(session.drain_events(160)?);
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].text, "final");
    Ok(())
}

#[test]
fn empty_completion_ends_the_unit_as_no_speech() -> AppResult<()> {
    let (mut session, probe) = session(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut session, "unit-a", 100)?;
    session.end_input()?;
    probe.push_server_event(buffer_committed("item-a"))?;
    probe.push_server_event(transcript_completed("item-a", "  "))?;

    let events = session.drain_events(160)?;
    assert!(matches!(
        events.as_slice(),
        [RecognitionEvent::UnitEnded {
            unit_id,
            reason: RecognitionEndReason::NoSpeech,
            ..
        }] if unit_id == "unit-a"
    ));
    Ok(())
}

#[test]
fn failed_first_item_ends_explicitly_then_releases_the_completed_second_item() -> AppResult<()> {
    let (mut session, probe) = session(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut session, "unit-a", 100)?;
    session.end_input()?;
    start_unit(&mut session, "unit-b", 200)?;
    session.end_input()?;
    probe.push_server_event(buffer_committed("item-a"))?;
    probe.push_server_event(buffer_committed("item-b"))?;
    probe.push_server_event(transcript_completed("item-b", "second"))?;
    probe.push_server_event(transcript_failed("item-a", "recognition failed"))?;

    let events = session.drain_events(280)?;
    assert!(matches!(
        events.first(),
        Some(RecognitionEvent::UnitEnded {
            unit_id,
            reason: RecognitionEndReason::Failed { detail },
            ..
        }) if unit_id == "unit-a" && detail == "OpenAI could not transcribe one audio item."
    ));
    assert!(matches!(
        events.get(1),
        Some(RecognitionEvent::Caption(caption))
            if caption.unit_id.as_deref() == Some("unit-b") && caption.text == "second"
    ));
    Ok(())
}

#[test]
fn structured_item_failures_promote_session_wide_conditions_to_clean_failure() -> AppResult<()> {
    for (kind, code, expected_class, expected_retry) in [
        (
            "rate_limit_error",
            "rate_limit_exceeded",
            ProviderFailureClass::RateLimited,
            RetryDisposition::Retryable,
        ),
        (
            "insufficient_quota",
            "insufficient_quota",
            ProviderFailureClass::UsageLimit,
            RetryDisposition::Terminal,
        ),
    ] {
        let (mut session, probe) = session(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
        start_unit(&mut session, "unit-a", 100)?;
        session.end_input()?;
        probe.push_server_event(buffer_committed("item-a"))?;
        probe.push_server_event(json!({
            "type": "conversation.item.input_audio_transcription.failed",
            "item_id": "item-a",
            "error": {
                "message": "provider-item-message-canary",
                "type": kind,
                "code": code,
            }
        }))?;

        let error = session.drain_events(180).err().ok_or_else(|| {
            AppError::state("A session-wide item failure unexpectedly succeeded.")
        })?;
        assert_eq!(error.provider_failure_class(), Some(expected_class));
        assert_eq!(error.retry_disposition(), expected_retry);
        assert!(!format!("{error:?}\n{error}").contains("provider-item-message-canary"));
        assert_eq!(probe.close_count()?, 1);
    }
    Ok(())
}

#[test]
fn item_failure_does_not_escape_through_recognition_events_or_tracing() -> AppResult<()> {
    let (mut session, probe) = session(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut session, "unit-a", 100)?;
    session.end_input()?;
    let canaries = [
        "item-id-canary",
        "item-message-canary",
        "item-type-canary",
        "item-code-canary",
        "item-param-canary",
        "item-event-id-canary",
    ];
    probe.push_server_event(buffer_committed(canaries[0]))?;
    probe.push_server_event(json!({
        "type": "conversation.item.input_audio_transcription.failed",
        "item_id": canaries[0],
        "error": {
            "message": canaries[1],
            "type": canaries[2],
            "code": canaries[3],
            "param": canaries[4],
            "event_id": canaries[5],
        }
    }))?;

    let capture = TracingCapture::default();
    let writer_capture = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_writer(move || writer_capture.writer())
        .finish();
    let events = tracing::subscriber::with_default(subscriber, || session.drain_events(180))?;

    assert!(matches!(
        events.as_slice(),
        [RecognitionEvent::UnitEnded {
            unit_id,
            reason: RecognitionEndReason::Failed { detail },
            ..
        }] if unit_id == "unit-a"
            && detail == "OpenAI could not transcribe one audio item."
    ));
    let observable = format!("{events:?}\n{}", capture.contents()?);
    for canary in canaries {
        assert!(!observable.contains(canary));
    }
    Ok(())
}

#[test]
fn a_timed_out_item_ends_explicitly_and_releases_later_completed_items() -> AppResult<()> {
    let (mut session, probe, clock) =
        session_with_manual_clock(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut session, "unit-a", 100)?;
    session.end_input()?;
    start_unit(&mut session, "unit-b", 200)?;
    session.end_input()?;
    probe.push_server_event(buffer_committed("item-a"))?;
    probe.push_server_event(buffer_committed("item-b"))?;
    probe.push_server_event(transcript_completed("item-b", "second"))?;

    assert!(session.drain_events(1_000)?.is_empty());
    clock.advance_ms(29_999)?;
    assert!(session.drain_events(1_001)?.is_empty());
    clock.advance_ms(1)?;
    let events = session.drain_events(1_002)?;

    assert!(matches!(
        events.first(),
        Some(RecognitionEvent::UnitEnded {
            unit_id,
            reason: RecognitionEndReason::Failed { detail },
            ..
        }) if unit_id == "unit-a" && detail.contains("did not complete")
    ));
    assert!(matches!(
        events.get(1),
        Some(RecognitionEvent::Caption(caption))
            if caption.unit_id.as_deref() == Some("unit-b") && caption.text == "second"
    ));
    Ok(())
}

#[test]
fn wall_clock_jump_forward_does_not_expire_a_committed_item() -> AppResult<()> {
    let (mut session, probe, _clock) =
        session_with_manual_clock(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut session, "unit-a", 100)?;
    session.end_input()?;
    probe.push_server_event(buffer_committed("item-a"))?;

    assert!(session.drain_events(1_000)?.is_empty());
    assert!(session.drain_events(u64::MAX)?.is_empty());
    Ok(())
}

#[test]
fn wall_clock_jump_backward_does_not_delay_a_committed_item_timeout() -> AppResult<()> {
    let (mut session, probe, clock) =
        session_with_manual_clock(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut session, "unit-a", 100)?;
    session.end_input()?;
    probe.push_server_event(buffer_committed("item-a"))?;

    assert!(session.drain_events(u64::MAX)?.is_empty());
    clock.advance_ms(30_000)?;
    let events = session.drain_events(0)?;
    assert!(matches!(
        events.as_slice(),
        [RecognitionEvent::UnitEnded {
            unit_id,
            reason: RecognitionEndReason::Failed { detail },
            ..
        }] if unit_id == "unit-a" && detail.contains("did not complete")
    ));
    Ok(())
}

#[test]
fn an_unacknowledged_commit_times_out_instead_of_misbinding_a_later_item() -> AppResult<()> {
    let (mut session, _probe, clock) =
        session_with_manual_clock(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut session, "unit-a", 100)?;
    session.end_input()?;

    clock.advance_ms(30_000)?;
    let error = session
        .drain_events(1_000)
        .err()
        .ok_or_else(|| AppError::state("An unacknowledged commit never timed out."))?;

    assert!(error.to_string().contains("did not acknowledge"));
    assert!(error.to_string().contains("reconnect"));
    assert_eq!(error.retry_disposition(), RetryDisposition::Retryable);
    Ok(())
}

#[test]
fn a_saturated_provider_stream_cannot_postpone_an_overdue_item_forever() -> AppResult<()> {
    let (mut session, probe, clock) =
        session_with_manual_clock(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut session, "unit-a", 100)?;
    session.end_input()?;
    probe.push_server_event(buffer_committed("item-a"))?;
    assert!(session.drain_events(1_000)?.is_empty());

    for _ in 0..(MAX_SERVER_FRAMES_PER_DRAIN * 2) {
        probe.push_server_event(json!({ "type": "provider.keepalive" }))?;
    }
    clock.advance_ms(30_000)?;
    assert!(session.drain_events(1_001)?.is_empty());
    let events = session.drain_events(1_002)?;

    assert!(matches!(
        events.as_slice(),
        [RecognitionEvent::UnitEnded {
            unit_id,
            reason: RecognitionEndReason::Failed { detail },
            ..
        }] if unit_id == "unit-a"
            && detail.contains("did not complete")
            && !detail.contains("item-a")
    ));
    Ok(())
}

#[test]
fn stop_closes_once_and_permanently_suppresses_queued_provider_output() -> AppResult<()> {
    let (mut session, probe) = session(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut session, "unit-a", 100)?;
    session.end_input()?;
    probe.push_server_event(buffer_committed("item-a"))?;
    probe.push_server_event(transcript_completed("item-a", "too late"))?;

    session.stop()?;
    session.stop()?;

    assert!(session.drain_events(200)?.is_empty());
    assert_eq!(probe.close_count()?, 1);
    assert!(session.start_unit("unit-b".to_string(), 220).is_err());
    Ok(())
}
