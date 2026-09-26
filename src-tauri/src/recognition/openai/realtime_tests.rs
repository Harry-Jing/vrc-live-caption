use super::super::attempt::{RecognitionAttempt, RecognitionAttemptAudioChunk};
use super::super::{OpenAiRecognitionAttemptFactory, OpenAiRecognitionDriver};
use super::*;
use crate::caption::{CaptionSnapshot, CaptionState};
use crate::error::{AppError, AppResult, ProviderFailureClass, RetryDisposition};
use crate::recognition::{
    OwnedRecognitionAudioFrame, RecognitionEvent, RecognitionGenerationScope, RecognitionModule,
    RecognitionSignal, RecognitionUnitAbortReason, RunningRecognition,
};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// The exact event OpenAI sends when a Realtime session reaches its maximum
/// duration; the server closes the socket after it.
const SESSION_EXPIRED_EVENT: &str = r#"{"type":"error","error":{"type":"invalid_request_error","code":"session_expired","message":"Your session hit the maximum duration of 60 minutes.","param":null,"event_id":null}}"#;
const SESSION_EXPIRED_PROVIDER_MESSAGE: &str =
    "Your session hit the maximum duration of 60 minutes.";
const DRIVER_SIGNAL_WATCHDOG: Duration = Duration::from_secs(2);

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

    fn unread_server_events(&self) -> AppResult<usize> {
        Ok(self.lock()?.received.len())
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

/// Captures this thread's tracing output while `scenario` runs.
///
/// `tracing` caches callsite interest process-wide and, while at most one
/// dispatcher exists, computes it on the thread that registers the callsite
/// first. A concurrently running test without a subscriber can therefore cache
/// `never` for the provider-failure callsite. Registering that callsite first
/// under a throwaway subscriber lets the capture subscriber's own registration
/// rebuild its interest.
fn capture_provider_failure_tracing<T>(scenario: impl FnOnce() -> T) -> AppResult<(T, String)> {
    let registration = tracing_subscriber::fmt().with_writer(io::sink).finish();
    tracing::subscriber::with_default(registration, || -> AppResult<()> {
        let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
        probe.push_server_event(json!({ "type": "error", "error": {} }))?;
        if attempt.drain_events(0).is_ok() {
            return Err(AppError::state(
                "The provider-failure callsite registration did not fail.",
            ));
        }
        Ok(())
    })?;

    let capture = TracingCapture::default();
    let writer_capture = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_writer(move || writer_capture.writer())
        .finish();
    let result = tracing::subscriber::with_default(subscriber, scenario);
    Ok((result, capture.contents()?))
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

fn attempt(
    model: OpenAiTranscriptionModel,
    languages: &[&str],
) -> AppResult<(OpenAiRealtimeAttempt<FakeTransport>, FakeTransportProbe)> {
    let (transport, probe) = FakeTransport::new();
    let attempt = OpenAiRealtimeAttempt::connect(
        OpenAiRealtimeAttemptContext {
            generation: 7,
            connection_epoch: 3,
            stream_id: "recognition-7-1".to_string(),
        },
        model,
        languages.iter().map(|value| (*value).to_string()).collect(),
        transport,
    )?;
    Ok((attempt, probe))
}

fn attempt_with_manual_clock(
    model: OpenAiTranscriptionModel,
    languages: &[&str],
) -> AppResult<(
    OpenAiRealtimeAttempt<FakeTransport>,
    FakeTransportProbe,
    ManualClock,
)> {
    let (transport, probe) = FakeTransport::new();
    let clock = ManualClock::default();
    let attempt = OpenAiRealtimeAttempt::connect_with_clock(
        OpenAiRealtimeAttemptContext {
            generation: 7,
            connection_epoch: 3,
            stream_id: "recognition-7-1".to_string(),
        },
        model,
        languages.iter().map(|value| (*value).to_string()).collect(),
        transport,
        Box::new(clock.clone()),
    )?;
    Ok((attempt, probe, clock))
}

type ScriptedSessions = Arc<Mutex<Vec<(OpenAiRealtimeAttemptContext, FakeTransportProbe)>>>;

/// Opens every Driver attempt as a confirmed session over its own fake
/// transport and records it, so a test can script one specific session.
struct FakeTransportAttemptFactory {
    sessions: ScriptedSessions,
}

impl OpenAiRecognitionAttemptFactory for FakeTransportAttemptFactory {
    type Attempt = OpenAiRealtimeAttempt<FakeTransport>;

    fn connect(
        &mut self,
        context: OpenAiRealtimeAttemptContext,
        _is_cancelled: &dyn Fn() -> bool,
    ) -> AppResult<Self::Attempt> {
        let (transport, probe) = FakeTransport::new();
        probe.push_server_event(json!({ "type": "session.updated", "session": {} }))?;
        // The manual clock keeps the item-completion deadline out of Driver tests.
        let mut attempt = OpenAiRealtimeAttempt::connect_with_clock(
            context.clone(),
            OpenAiTranscriptionModel::GptLiveTranscribe,
            vec!["en".to_string()],
            transport,
            Box::new(ManualClock::default()),
        )?;
        if !attempt.drain_events(0)?.is_empty() || !attempt.is_ready() {
            return Err(AppError::state(
                "A scripted Realtime session did not confirm its configuration.",
            ));
        }
        self.sessions
            .lock()
            .map_err(|_| AppError::state("Scripted Realtime session lock was poisoned."))?
            .push((context, probe));
        Ok(attempt)
    }
}

fn scripted_session(sessions: &ScriptedSessions, index: usize) -> AppResult<FakeTransportProbe> {
    sessions
        .lock()
        .map_err(|_| AppError::state("Scripted Realtime session lock was poisoned."))?
        .get(index)
        .map(|(_, probe)| probe.clone())
        .ok_or_else(|| AppError::state(format!("Scripted Realtime session {index} never opened.")))
}

fn next_driver_signal(
    running: &RunningRecognition,
    observed: &mut Vec<RecognitionSignal>,
) -> AppResult<RecognitionSignal> {
    let signal = running
        .signals
        .recv_timeout(DRIVER_SIGNAL_WATCHDOG)
        .map_err(|error| {
            AppError::state(format!(
                "The Recognition Driver emitted no signal before the watchdog: {error:?}"
            ))
        })?;
    observed.push(signal.clone());
    Ok(signal)
}

fn submit_speech(
    running: &RunningRecognition,
    sequence: u64,
    captured_at_ms: u64,
) -> AppResult<()> {
    running
        .try_submit(OwnedRecognitionAudioFrame {
            sequence,
            captured_at_ms,
            sample_rate_hz: 16_000,
            samples: vec![0.25; 4_800].into_boxed_slice(),
        })
        .map_err(|error| AppError::state(format!("Test speech was rejected: {error:?}")))
}

fn start_unit(
    attempt: &mut impl RecognitionAttempt,
    unit_id: &str,
    started_at_ms: u64,
) -> AppResult<()> {
    let event = attempt.start_unit(unit_id.to_string(), started_at_ms)?;
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

fn captions(events: Vec<RecognitionEvent>) -> Vec<CaptionSnapshot> {
    events
        .into_iter()
        .filter_map(|event| match event {
            RecognitionEvent::Caption(caption) => Some(caption),
            RecognitionEvent::UnitStarted { .. } | RecognitionEvent::UnitAborted { .. } => None,
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
fn connection_configures_a_transcription_session_with_pcm_24k_and_languages() -> AppResult<()> {
    let (_attempt, probe) = attempt(OpenAiTranscriptionModel::GptLiveTranscribe, &["en", "zh"])?;
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
fn attempt_is_not_ready_until_openai_confirms_the_session_update() -> AppResult<()> {
    let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    assert!(!attempt.is_ready());

    probe.push_server_event(json!({ "type": "session.updated", "session": {} }))?;
    assert!(attempt.drain_events(10)?.is_empty());

    assert!(attempt.is_ready());
    Ok(())
}

#[test]
fn append_encodes_mono_pcm16_at_24k_then_commit_is_a_separate_event() -> AppResult<()> {
    let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut attempt, "unit-a", 100)?;

    attempt.append_audio(RecognitionAttemptAudioChunk {
        sample_rate_hz: 24_000,
        samples: &[0.0, 1.0, -1.0],
    })?;
    attempt.end_input()?;

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
    let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
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

    let error = attempt
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
    let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
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

    let error = attempt
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
        (
            "invalid_request_error",
            Some("session_expired"),
            ProviderFailureClass::SessionExpired,
            RetryDisposition::Retryable,
            "stt.provider_session_expired",
        ),
    ];

    for (kind, code, expected_class, expected_retry, expected_code) in cases {
        let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
        probe.push_server_event(json!({
            "type": "error",
            "error": {
                "type": kind,
                "code": code,
                "message": "invalid_request_error must not control this classification",
            }
        }))?;

        let error = attempt
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
    let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
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

    let (result, tracing_output) = capture_provider_failure_tracing(|| attempt.drain_events(100))?;
    assert!(result.is_err());
    assert!(tracing_output.contains("OpenAI Realtime provider failure"));
    for canary in canaries {
        assert!(!tracing_output.contains(canary));
    }
    Ok(())
}

/// Our code contains the provider's code as a substring, so it is removed
/// before checking that no provider-authored field was echoed.
fn assert_no_session_expired_provider_text(observable: &str) {
    assert!(!observable.contains(SESSION_EXPIRED_PROVIDER_MESSAGE));
    assert!(!observable.contains("invalid_request_error"));
    assert!(
        !observable
            .replace("stt.provider_session_expired", "")
            .contains("session_expired")
    );
}

#[test]
fn session_expired_code_is_classified_before_its_invalid_request_type() -> AppResult<()> {
    let event = serde_json::from_str::<ServerEvent>(SESSION_EXPIRED_EVENT).map_err(|error| {
        AppError::state(format!(
            "Session-expiry fixture was not a server event: {error}"
        ))
    })?;
    let ServerEvent::Error {
        error: provider_error,
    } = event
    else {
        return Err(AppError::state(
            "Session-expiry fixture did not decode as a provider error event.",
        ));
    };
    assert_eq!(
        provider_error.classification(),
        ProviderFailureClass::SessionExpired
    );

    // Unrecognized codes keep falling back to the broader type as before.
    for (provider_error, expected_class) in [
        (
            json!({ "code": "session_expired" }),
            ProviderFailureClass::SessionExpired,
        ),
        (
            json!({ "type": "invalid_request_error", "code": "unrecognized_code" }),
            ProviderFailureClass::InvalidRequest,
        ),
        (
            json!({ "type": "server_error", "code": "unrecognized_code" }),
            ProviderFailureClass::ServiceUnavailable,
        ),
        (
            json!({ "code": "unrecognized_code" }),
            ProviderFailureClass::Unknown,
        ),
    ] {
        let provider_error =
            serde_json::from_value::<ProviderError>(provider_error).map_err(|error| {
                AppError::state(format!("Provider error fixture did not decode: {error}"))
            })?;
        assert_eq!(provider_error.classification(), expected_class);
    }
    Ok(())
}

#[test]
fn session_expiry_retires_the_attempt_once_as_a_retryable_failure() -> AppResult<()> {
    let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptLiveTranscribe, &["en"])?;
    start_unit(&mut attempt, "unit-a", 100)?;
    attempt.append_audio(RecognitionAttemptAudioChunk {
        sample_rate_hz: 24_000,
        samples: &[0.25; 240],
    })?;
    probe.push_raw_server_event(SESSION_EXPIRED_EVENT)?;
    // Whatever follows the notice belongs to the retired session. A frame that
    // would be terminal on its own proves that nothing is classified twice.
    probe.push_server_event(json!({
        "type": "error",
        "error": { "type": "authentication_error", "code": "invalid_api_key" },
    }))?;

    let error = attempt
        .drain_events(200)
        .err()
        .ok_or_else(|| AppError::state("Session expiry unexpectedly succeeded."))?;
    let serialized = serde_json::to_string(&error)
        .map_err(|error| AppError::state(format!("Failed to serialize provider error: {error}")))?;

    assert_eq!(
        error.provider_failure_class(),
        Some(ProviderFailureClass::SessionExpired)
    );
    assert_eq!(error.retry_disposition(), RetryDisposition::Retryable);
    assert_eq!(error.code(), "stt.provider_session_expired");
    assert_eq!(
        error.to_string(),
        "The OpenAI Realtime session reached its maximum duration; reconnecting."
    );
    assert_no_session_expired_provider_text(&format!("{error:?}\n{error}\n{serialized}"));
    assert!(attempt.drain_events(220)?.is_empty());
    assert_eq!(probe.close_count()?, 1);
    assert_eq!(probe.unread_server_events()?, 1);
    Ok(())
}

#[test]
fn session_expiry_diagnostics_carry_the_application_code_without_provider_text() -> AppResult<()> {
    let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    probe.push_raw_server_event(SESSION_EXPIRED_EVENT)?;

    let (result, tracing_output) = capture_provider_failure_tracing(|| attempt.drain_events(100))?;
    assert!(result.is_err());
    assert!(tracing_output.contains("code=\"stt.provider_session_expired\""));
    assert!(tracing_output.contains("provider_failure_class=Some(SessionExpired)"));
    assert!(tracing_output.contains("retry_disposition=Retryable"));
    assert_no_session_expired_provider_text(&tracing_output);
    Ok(())
}

#[test]
fn malformed_provider_metadata_is_discarded_without_entering_parser_errors() -> AppResult<()> {
    let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
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

    let error = attempt
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
    let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    let canary = "provider-message-canary";
    probe.push_raw_server_event(format!(r#"{{"type":"error","error":"{canary}"}}"#))?;

    let error = attempt
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
    let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut attempt, "unit-a", 100)?;
    attempt.end_input()?;

    probe.push_server_event(buffer_committed("item-a"))?;
    probe.push_server_event(transcript_delta("item-a", "not downstream"))?;
    probe.push_server_event(transcript_completed("item-a", "final transcript"))?;

    let captions = captions(attempt.drain_events(180)?);
    assert_eq!(captions.len(), 1);
    assert_eq!(captions[0].unit_id.as_deref(), Some("unit-a"));
    assert_eq!(captions[0].text, "final transcript");
    assert_eq!(captions[0].revision, 1);
    assert_eq!(captions[0].state, CaptionState::Completed);
    assert_eq!(captions[0].language, None);
    assert_eq!(captions[0].unit_started_at_ms, Some(100));
    assert_eq!(captions[0].timestamp_ms, 180);
    Ok(())
}

#[test]
fn gpt_live_transcribe_emits_full_ongoing_snapshots_before_commit_and_a_final() -> AppResult<()> {
    let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptLiveTranscribe, &["en"])?;
    start_unit(&mut attempt, "unit-live", 200)?;

    probe.push_server_event(transcript_delta("item-live", "hello"))?;
    probe.push_server_event(transcript_delta("item-live", " world"))?;
    let ongoing = captions(attempt.drain_events(240)?);
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

    attempt.end_input()?;
    probe.push_server_event(buffer_committed("item-live"))?;
    probe.push_server_event(transcript_completed("item-live", "Hello world."))?;
    let completed = captions(attempt.drain_events(300)?);
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].text, "Hello world.");
    assert_eq!(completed[0].revision, 3);
    assert_eq!(completed[0].state, CaptionState::Completed);
    assert_eq!(completed[0].language, None);
    Ok(())
}

#[test]
fn live_item_binding_replays_earlier_delta_before_the_current_delta() -> AppResult<()> {
    let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptLiveTranscribe, &["en"])?;
    start_unit(&mut attempt, "unit-a", 100)?;
    attempt.end_input()?;
    start_unit(&mut attempt, "unit-b", 120)?;

    // Unit A is committed but not yet bound, so B's first early delta cannot
    // be attached safely and must wait for A's provider item binding.
    probe.push_server_event(transcript_delta("item-b", "hello"))?;
    assert!(attempt.drain_events(130)?.is_empty());

    probe.push_server_event(buffer_committed("item-a"))?;
    probe.push_server_event(transcript_delta("item-b", " world"))?;
    let captions = captions(attempt.drain_events(140)?);

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
    let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["en", "fr"])?;
    start_unit(&mut attempt, "unit-a", 100)?;
    attempt.end_input()?;
    probe.push_server_event(buffer_committed("item-a"))?;
    probe.push_server_event(transcript_completed_with_languages(
        "item-a",
        "Bonjour.",
        &["fr"],
    ))?;

    let captions = captions(attempt.drain_events(160)?);
    assert_eq!(captions.len(), 1);
    assert_eq!(captions[0].language.as_deref(), Some("fr"));
    Ok(())
}

#[test]
fn multiple_detected_languages_are_not_collapsed_into_a_singular_label() -> AppResult<()> {
    let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["zh", "en"])?;
    start_unit(&mut attempt, "unit-a", 100)?;
    attempt.end_input()?;
    probe.push_server_event(buffer_committed("item-a"))?;
    probe.push_server_event(transcript_completed_with_languages(
        "item-a",
        "你好, world.",
        &["zh", "en"],
    ))?;

    let captions = captions(attempt.drain_events(160)?);
    assert_eq!(captions.len(), 1);
    assert_eq!(captions[0].language, None);
    Ok(())
}

#[test]
fn outstanding_units_are_bounded_without_dropping_an_existing_unit() -> AppResult<()> {
    let (mut attempt, _probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;

    for index in 0..MAX_OUTSTANDING_UNITS {
        start_unit(&mut attempt, &format!("unit-{index}"), index as u64)?;
        attempt.end_input()?;
    }

    let error = attempt
        .start_unit("unit-overflow".to_string(), 999)
        .err()
        .ok_or_else(|| AppError::state("An unbounded recognition unit unexpectedly started."))?;
    assert!(error.to_string().contains("outstanding recognition units"));
    Ok(())
}

#[test]
fn uncorrelated_provider_items_and_events_are_bounded() -> AppResult<()> {
    let (mut item_attempt, item_probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;

    for index in 0..MAX_PENDING_PROVIDER_ITEMS {
        item_probe.push_server_event(transcript_completed(&format!("item-{index}"), "pending"))?;
    }
    item_probe.push_server_event(transcript_completed("item-overflow", "pending"))?;
    let item_error = item_attempt
        .drain_events(100)
        .err()
        .ok_or_else(|| AppError::state("Unbounded provider items were unexpectedly accepted."))?;
    assert!(
        item_error
            .to_string()
            .contains("uncorrelated provider items")
    );

    let (mut event_attempt, event_probe) =
        attempt(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    for _ in 0..=MAX_PENDING_EVENTS_PER_ITEM {
        event_probe.push_server_event(transcript_completed("item-a", "pending"))?;
    }
    assert!(event_attempt.drain_events(120)?.is_empty());
    let event_error = event_attempt
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
fn provider_transcript_text_is_bounded_per_unit() -> AppResult<()> {
    let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut attempt, "unit-a", 100)?;
    attempt.end_input()?;
    probe.push_server_event(buffer_committed("item-a"))?;
    probe.push_server_event(transcript_completed(
        "item-a",
        &"x".repeat(MAX_TRANSCRIPT_BYTES_PER_UNIT + 1),
    ))?;

    let error = attempt
        .drain_events(160)
        .err()
        .ok_or_else(|| AppError::state("An oversized transcript unexpectedly succeeded."))?;
    assert!(error.to_string().contains("per-unit text limit"));
    Ok(())
}

#[test]
fn completed_items_are_released_in_local_unit_order_using_item_ids() -> AppResult<()> {
    let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut attempt, "unit-a", 100)?;
    attempt.end_input()?;
    start_unit(&mut attempt, "unit-b", 200)?;
    attempt.end_input()?;

    probe.push_server_event(buffer_committed("item-a"))?;
    probe.push_server_event(buffer_committed("item-b"))?;
    assert!(attempt.drain_events(220)?.is_empty());

    probe.push_server_event(transcript_completed("item-b", "second"))?;
    assert!(attempt.drain_events(260)?.is_empty());

    probe.push_server_event(transcript_completed("item-a", "first"))?;
    let completed = captions(attempt.drain_events(280)?);
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
    let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut attempt, "unit-a", 100)?;
    attempt.end_input()?;

    probe.push_server_event(transcript_completed("item-a", "final"))?;
    assert!(attempt.drain_events(140)?.is_empty());
    probe.push_server_event(buffer_committed("item-a"))?;
    let completed = captions(attempt.drain_events(160)?);
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].text, "final");
    Ok(())
}

#[test]
fn empty_completion_aborts_the_unit_as_no_speech() -> AppResult<()> {
    let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut attempt, "unit-a", 100)?;
    attempt.end_input()?;
    probe.push_server_event(buffer_committed("item-a"))?;
    probe.push_server_event(transcript_completed("item-a", "  "))?;

    let events = attempt.drain_events(160)?;
    assert!(matches!(
        events.as_slice(),
        [RecognitionEvent::UnitAborted {
            unit_id,
            reason: RecognitionUnitAbortReason::NoSpeech,
            ..
        }] if unit_id == "unit-a"
    ));
    Ok(())
}

#[test]
fn failed_first_item_aborts_explicitly_then_releases_the_completed_second_item() -> AppResult<()> {
    let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut attempt, "unit-a", 100)?;
    attempt.end_input()?;
    start_unit(&mut attempt, "unit-b", 200)?;
    attempt.end_input()?;
    probe.push_server_event(buffer_committed("item-a"))?;
    probe.push_server_event(buffer_committed("item-b"))?;
    probe.push_server_event(transcript_completed("item-b", "second"))?;
    probe.push_server_event(transcript_failed("item-a", "recognition failed"))?;

    let events = attempt.drain_events(280)?;
    assert!(matches!(
        events.first(),
        Some(RecognitionEvent::UnitAborted {
            unit_id,
            reason: RecognitionUnitAbortReason::Failed { detail },
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
fn structured_item_failures_promote_attempt_wide_conditions_to_clean_failure() -> AppResult<()> {
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
        let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
        start_unit(&mut attempt, "unit-a", 100)?;
        attempt.end_input()?;
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

        let error = attempt.drain_events(180).err().ok_or_else(|| {
            AppError::state("A attempt-wide item failure unexpectedly succeeded.")
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
    let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut attempt, "unit-a", 100)?;
    attempt.end_input()?;
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
    let events = tracing::subscriber::with_default(subscriber, || attempt.drain_events(180))?;

    assert!(matches!(
        events.as_slice(),
        [RecognitionEvent::UnitAborted {
            unit_id,
            reason: RecognitionUnitAbortReason::Failed { detail },
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
fn a_timed_out_item_aborts_explicitly_and_releases_later_completed_items() -> AppResult<()> {
    let (mut attempt, probe, clock) =
        attempt_with_manual_clock(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut attempt, "unit-a", 100)?;
    attempt.end_input()?;
    start_unit(&mut attempt, "unit-b", 200)?;
    attempt.end_input()?;
    probe.push_server_event(buffer_committed("item-a"))?;
    probe.push_server_event(buffer_committed("item-b"))?;
    probe.push_server_event(transcript_completed("item-b", "second"))?;

    assert!(attempt.drain_events(1_000)?.is_empty());
    clock.advance_ms(29_999)?;
    assert!(attempt.drain_events(1_001)?.is_empty());
    clock.advance_ms(1)?;
    let events = attempt.drain_events(1_002)?;

    assert!(matches!(
        events.first(),
        Some(RecognitionEvent::UnitAborted {
            unit_id,
            reason: RecognitionUnitAbortReason::Failed { detail },
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
    let (mut attempt, probe, _clock) =
        attempt_with_manual_clock(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut attempt, "unit-a", 100)?;
    attempt.end_input()?;
    probe.push_server_event(buffer_committed("item-a"))?;

    assert!(attempt.drain_events(1_000)?.is_empty());
    assert!(attempt.drain_events(u64::MAX)?.is_empty());
    Ok(())
}

#[test]
fn wall_clock_jump_backward_does_not_delay_a_committed_item_timeout() -> AppResult<()> {
    let (mut attempt, probe, clock) =
        attempt_with_manual_clock(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut attempt, "unit-a", 100)?;
    attempt.end_input()?;
    probe.push_server_event(buffer_committed("item-a"))?;

    assert!(attempt.drain_events(u64::MAX)?.is_empty());
    clock.advance_ms(30_000)?;
    let events = attempt.drain_events(0)?;
    assert!(matches!(
        events.as_slice(),
        [RecognitionEvent::UnitAborted {
            unit_id,
            reason: RecognitionUnitAbortReason::Failed { detail },
            ..
        }] if unit_id == "unit-a" && detail.contains("did not complete")
    ));
    Ok(())
}

#[test]
fn an_unacknowledged_commit_times_out_instead_of_misbinding_a_later_item() -> AppResult<()> {
    let (mut attempt, _probe, clock) =
        attempt_with_manual_clock(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut attempt, "unit-a", 100)?;
    attempt.end_input()?;

    clock.advance_ms(30_000)?;
    let error = attempt
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
    let (mut attempt, probe, clock) =
        attempt_with_manual_clock(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut attempt, "unit-a", 100)?;
    attempt.end_input()?;
    probe.push_server_event(buffer_committed("item-a"))?;
    assert!(attempt.drain_events(1_000)?.is_empty());

    for _ in 0..(MAX_SERVER_FRAMES_PER_DRAIN * 2) {
        probe.push_server_event(json!({ "type": "provider.keepalive" }))?;
    }
    clock.advance_ms(30_000)?;
    assert!(attempt.drain_events(1_001)?.is_empty());
    let events = attempt.drain_events(1_002)?;

    assert!(matches!(
        events.as_slice(),
        [RecognitionEvent::UnitAborted {
            unit_id,
            reason: RecognitionUnitAbortReason::Failed { detail },
            ..
        }] if unit_id == "unit-a"
            && detail.contains("did not complete")
            && !detail.contains("item-a")
    ));
    Ok(())
}

#[test]
fn stop_closes_once_and_permanently_suppresses_queued_provider_output() -> AppResult<()> {
    let (mut attempt, probe) = attempt(OpenAiTranscriptionModel::GptTranscribe, &["en"])?;
    start_unit(&mut attempt, "unit-a", 100)?;
    attempt.end_input()?;
    probe.push_server_event(buffer_committed("item-a"))?;
    probe.push_server_event(transcript_completed("item-a", "too late"))?;

    attempt.stop()?;
    attempt.stop()?;

    assert!(attempt.drain_events(200)?.is_empty());
    assert_eq!(probe.close_count()?, 1);
    assert!(attempt.start_unit("unit-b".to_string(), 220).is_err());
    Ok(())
}

#[test]
fn session_expiry_reconnects_within_the_same_generation_with_monotonic_units() -> AppResult<()> {
    let sessions = ScriptedSessions::default();
    let driver = OpenAiRecognitionDriver::new(FakeTransportAttemptFactory {
        sessions: Arc::clone(&sessions),
    });
    let module = RecognitionModule::with_audio_budget(Duration::from_millis(500), 8, driver)?;
    let mut running = module.start(RecognitionGenerationScope {
        generation: 31,
        stream_id: "recognition-31-1".to_string(),
    })?;
    let mut observed = Vec::new();

    let ready = next_driver_signal(&running, &mut observed)?;
    assert!(matches!(
        ready,
        RecognitionSignal::Ready {
            recovered: false,
            ..
        }
    ));
    submit_speech(&running, 1, 100)?;
    let first_unit = next_driver_signal(&running, &mut observed)?;
    assert!(matches!(
        &first_unit,
        RecognitionSignal::Event(RecognitionEvent::UnitStarted {
            generation: 31,
            unit_id,
            started_at_ms: 100,
            ..
        }) if unit_id == "unit-1"
    ));

    // The first session still owns an open unit when OpenAI expires it.
    scripted_session(&sessions, 0)?.push_raw_server_event(SESSION_EXPIRED_EVENT)?;
    let pause_epoch = match next_driver_signal(&running, &mut observed)? {
        RecognitionSignal::Reconnecting {
            epoch: first_session_epoch @ 1,
            retry_number: 1,
            delay_ms,
        } => {
            assert!((400..=600).contains(&delay_ms));
            first_session_epoch
        }
        signal => {
            return Err(AppError::state(format!(
                "Session expiry did not start one reconnect: {signal:?}"
            )));
        }
    };
    assert!(!running.is_accepting_audio());
    running.acknowledge_capture_paused(pause_epoch)?;

    let recovered = next_driver_signal(&running, &mut observed)?;
    assert!(matches!(
        &recovered,
        RecognitionSignal::Ready {
            generation: 31,
            stream_id,
            recovered: true,
        } if stream_id == "recognition-31-1"
    ));
    submit_speech(&running, 2, 2_000)?;
    let second_unit = next_driver_signal(&running, &mut observed)?;
    assert!(matches!(
        &second_unit,
        RecognitionSignal::Event(RecognitionEvent::UnitStarted {
            generation: 31,
            unit_id,
            started_at_ms: 2_000,
            ..
        }) if unit_id == "unit-2"
    ));

    // A clean Stop proves the Driver never ended the generation with a failure.
    running.stop()?;

    let sessions = sessions
        .lock()
        .map_err(|_| AppError::state("Scripted Realtime session lock was poisoned."))?;
    assert_eq!(
        sessions
            .iter()
            .map(|(context, _)| (
                context.generation,
                context.connection_epoch,
                context.stream_id.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![(31, 1, "recognition-31-1"), (31, 2, "recognition-31-1")]
    );
    for (_, probe) in sessions.iter() {
        assert_eq!(probe.close_count()?, 1);
    }
    assert!(!format!("{observed:?}").contains(SESSION_EXPIRED_PROVIDER_MESSAGE));
    Ok(())
}
