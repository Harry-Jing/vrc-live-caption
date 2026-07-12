//! Normalized runtime events emitted to the Vue frontend.
//!
//! These events are the UI-facing contract for status, transcripts, and
//! diagnostics. Provider-specific raw events should be normalized before they
//! reach this module so Vue components and output sinks do not depend on STT
//! provider protocols.
//!
//! Diagnostic `code` values are machine-readable and follow one naming
//! convention: `<category>.<detail>` in snake case, where the prefix equals
//! the serialized `DiagnosticCategory` of the event. This applies both to
//! codes written inline at emit sites and to codes from `AppError::code`.
//!
//! Event delivery is best-effort: Tauri events are at-most-once, so the UI
//! must already tolerate missed events, and emit failures are logged here
//! rather than propagated. The runtime's lifecycle never depends on whether
//! an event reached the webview.

use crate::error::AppError;
use crate::state::AppState;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, Runtime};

const EVENT_RUNTIME_STATUS: &str = "runtime-status";
const EVENT_TRANSCRIPT_PARTIAL: &str = "transcript-partial";
const EVENT_TRANSCRIPT_FINAL: &str = "transcript-final";
const EVENT_UTTERANCE_STARTED: &str = "utterance-started";
const EVENT_UTTERANCE_ENDED: &str = "utterance-ended";
const EVENT_DIAGNOSTIC: &str = "diagnostic-event";

static NEXT_EVENT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeStatusEvent {
    pub(crate) status: RuntimeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) message: Option<String>,
    pub(crate) timestamp_ms: u64,
}

impl RuntimeStatusEvent {
    pub(crate) fn idle() -> Self {
        Self::new(RuntimeStatus::Idle, Some("Runtime is idle".to_string()))
    }

    fn new(status: RuntimeStatus, message: Option<String>) -> Self {
        Self {
            status,
            message,
            timestamp_ms: now_ms(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RuntimeStatus {
    Idle,
    Starting,
    Running,
    Stopping,
    Stopped,
    Error,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TranscriptEvent {
    id: String,
    utterance_id: String,
    kind: TranscriptKind,
    text: String,
    language: String,
    provider: String,
    revision: u32,
    timestamp_ms: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum TranscriptKind {
    Partial,
    // Stable is part of the normalized transcript contract for future providers.
    #[expect(
        dead_code,
        reason = "stable transcript events are not emitted in the MVP yet"
    )]
    Stable,
    Final,
}

#[derive(Clone)]
pub(crate) struct TranscriptUpdate {
    pub(crate) utterance_id: String,
    pub(crate) text: String,
    pub(crate) language: String,
    pub(crate) provider: String,
    pub(crate) revision: u32,
}

/// Start of a confirmed utterance. Emitted before any transcript text exists,
/// so transcript events never carry placeholder text such as a listening
/// indicator.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UtteranceStartedEvent {
    id: String,
    utterance_id: String,
    timestamp_ms: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UtteranceEndedEvent {
    id: String,
    utterance_id: String,
    reason: UtteranceEndReason,
    timestamp_ms: u64,
}

/// Why an utterance terminated without a final transcript. Successful
/// utterances end with `transcript-final` instead of this event.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum UtteranceEndReason {
    /// STT finished but recognized no words.
    NoSpeech,
    /// The STT request failed; details arrive as a diagnostic event.
    SttFailed,
    /// The captured segment was dropped before reaching STT.
    Discarded,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticEvent {
    id: String,
    category: DiagnosticCategory,
    severity: DiagnosticSeverity,
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    timestamp_ms: u64,
}

pub(crate) struct DiagnosticUpdate {
    category: DiagnosticCategory,
    severity: DiagnosticSeverity,
    code: &'static str,
    message: String,
    detail: Option<String>,
}

impl DiagnosticUpdate {
    pub(crate) fn info(
        category: DiagnosticCategory,
        code: &'static str,
        message: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self::with_severity(category, DiagnosticSeverity::Info, code, message, detail)
    }

    pub(crate) fn warning(
        category: DiagnosticCategory,
        code: &'static str,
        message: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self::with_severity(category, DiagnosticSeverity::Warning, code, message, detail)
    }

    pub(crate) fn error(
        category: DiagnosticCategory,
        code: &'static str,
        message: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self::with_severity(category, DiagnosticSeverity::Error, code, message, detail)
    }

    /// Error diagnostic with the category, code, and detail derived from the
    /// failure itself; `message` describes the operation that failed.
    pub(crate) fn from_error(error: &AppError, message: impl Into<String>) -> Self {
        Self {
            category: DiagnosticCategory::for_error(error),
            severity: DiagnosticSeverity::Error,
            code: error.code(),
            message: message.into(),
            detail: Some(error.to_string()),
        }
    }

    fn with_severity(
        category: DiagnosticCategory,
        severity: DiagnosticSeverity,
        code: &'static str,
        message: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            category,
            severity,
            code,
            message: message.into(),
            detail: Some(detail.into()),
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DiagnosticCategory {
    Audio,
    Config,
    Osc,
    Runtime,
    Stt,
}

impl DiagnosticCategory {
    /// Category for diagnostics built from an `AppError`. The match is
    /// exhaustive on purpose: adding an error variant must force an explicit
    /// category decision here instead of falling back silently.
    pub(crate) fn for_error(error: &AppError) -> Self {
        match error {
            AppError::Audio { .. } => Self::Audio,
            AppError::Config { .. } | AppError::ConfigIo { .. } | AppError::Secret { .. } => {
                Self::Config
            }
            AppError::OscEncode { .. }
            | AppError::OscBind { .. }
            | AppError::OscSend { .. }
            | AppError::OscSendIncomplete { .. } => Self::Osc,
            AppError::Runtime { .. } | AppError::State { .. } => Self::Runtime,
            AppError::Stt { .. } | AppError::Wav { .. } => Self::Stt,
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DiagnosticSeverity {
    Error,
    Info,
    Warning,
}

pub(crate) fn emit_status<R: Runtime>(
    app: &AppHandle<R>,
    status: RuntimeStatus,
    message: Option<String>,
) {
    let event = RuntimeStatusEvent::new(status, message);

    // Update the pull-side snapshot before best-effort delivery. If the
    // webview is reloading and misses this emit, its next status query still
    // observes the lifecycle transition.
    match app.try_state::<AppState>() {
        Some(state) => {
            if let Err(error) = state.runtime.replace_status(event.clone()) {
                tracing::warn!(
                    code = error.code(),
                    error_message = %error,
                    "failed to update runtime status snapshot"
                );
            }
        }
        None => {
            tracing::warn!("runtime status emitted before app state was managed");
        }
    }

    emit_event(app, EVENT_RUNTIME_STATUS, event);
}

pub(crate) fn emit_transcript_partial<R: Runtime>(app: &AppHandle<R>, update: TranscriptUpdate) {
    emit_transcript(
        app,
        EVENT_TRANSCRIPT_PARTIAL,
        TranscriptKind::Partial,
        update,
    );
}

pub(crate) fn emit_transcript_final<R: Runtime>(app: &AppHandle<R>, update: TranscriptUpdate) {
    emit_transcript(app, EVENT_TRANSCRIPT_FINAL, TranscriptKind::Final, update);
}

pub(crate) fn emit_utterance_started<R: Runtime>(app: &AppHandle<R>, utterance_id: String) {
    emit_event(
        app,
        EVENT_UTTERANCE_STARTED,
        UtteranceStartedEvent {
            id: next_event_id("utterance-start"),
            utterance_id,
            timestamp_ms: now_ms(),
        },
    );
}

pub(crate) fn emit_utterance_ended<R: Runtime>(
    app: &AppHandle<R>,
    utterance_id: String,
    reason: UtteranceEndReason,
) {
    emit_event(
        app,
        EVENT_UTTERANCE_ENDED,
        UtteranceEndedEvent {
            id: next_event_id("utterance-end"),
            utterance_id,
            reason,
            timestamp_ms: now_ms(),
        },
    );
}

pub(crate) fn emit_diagnostic<R: Runtime>(app: &AppHandle<R>, update: DiagnosticUpdate) {
    emit_event(
        app,
        EVENT_DIAGNOSTIC,
        DiagnosticEvent {
            id: next_event_id("diagnostic"),
            category: update.category,
            severity: update.severity,
            code: update.code,
            message: update.message,
            detail: update.detail,
            timestamp_ms: now_ms(),
        },
    );
}

pub(crate) fn next_utterance_id(prefix: &str) -> String {
    next_event_id(prefix)
}

fn emit_transcript<R: Runtime>(
    app: &AppHandle<R>,
    event_name: &str,
    kind: TranscriptKind,
    update: TranscriptUpdate,
) {
    emit_event(
        app,
        event_name,
        TranscriptEvent {
            id: next_event_id("transcript"),
            utterance_id: update.utterance_id,
            kind,
            text: update.text,
            language: update.language,
            provider: update.provider,
            revision: update.revision,
            timestamp_ms: now_ms(),
        },
    );
}

/// Emission is best-effort by design: Tauri events are at-most-once with no
/// acknowledgement, and in practice an emit only fails while the webview is
/// being torn down. No caller can act on such a failure, so it is logged here
/// instead of being propagated.
fn emit_event<R: Runtime, P: Serialize + Clone>(app: &AppHandle<R>, event_name: &str, payload: P) {
    if let Err(error) = app.emit(event_name, payload) {
        tracing::warn!(
            event_name,
            error_message = %error,
            "failed to emit runtime event"
        );
    }
}

fn next_event_id(prefix: &str) -> String {
    let sequence = NEXT_EVENT_ID.fetch_add(1, Ordering::Relaxed);

    format!("{prefix}-{}-{sequence}", now_ms())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppResult;
    use tauri::Listener;

    #[test]
    fn status_snapshot_is_updated_before_event_delivery() -> AppResult<()> {
        let app = tauri::test::mock_builder()
            .manage(AppState::default())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .map_err(|error| AppError::runtime(format!("Failed to build test app: {error}")))?;
        let listener_handle = app.handle().clone();
        let (snapshot_sender, snapshot_receiver) = std::sync::mpsc::channel();

        app.listen(EVENT_RUNTIME_STATUS, move |_| {
            let snapshot = listener_handle
                .state::<AppState>()
                .runtime
                .status_snapshot()
                .and_then(|status| {
                    serde_json::to_value(status).map_err(|error| {
                        AppError::runtime(format!("Failed to serialize status snapshot: {error}"))
                    })
                });
            let _ = snapshot_sender.send(snapshot);
        });

        emit_status(
            app.handle(),
            RuntimeStatus::Running,
            Some("Listening for microphone speech".to_string()),
        );

        let snapshot = snapshot_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Runtime status event was not delivered."))??;
        assert_eq!(snapshot["status"], "running");
        assert_eq!(snapshot["message"], "Listening for microphone speech");

        Ok(())
    }

    #[test]
    fn transcript_payload_includes_stable_runtime_contract_fields() {
        let event = TranscriptEvent {
            id: "event-1".to_string(),
            utterance_id: "utterance-1".to_string(),
            kind: TranscriptKind::Partial,
            text: "hello".to_string(),
            language: "en-US".to_string(),
            provider: "mock".to_string(),
            revision: 1,
            timestamp_ms: 42,
        };
        let value = serde_json::to_value(event).unwrap_or_else(|serialization_error| {
            serde_json::json!({ "serializationError": serialization_error.to_string() })
        });

        assert_eq!(value["id"], "event-1");
        assert_eq!(value["utteranceId"], "utterance-1");
        assert_eq!(value["kind"], "partial");
        assert_eq!(value["language"], "en-US");
        assert_eq!(value["provider"], "mock");
        assert_eq!(value["revision"], 1);
    }

    #[test]
    fn utterance_started_payload_uses_stable_wire_format() {
        let event = UtteranceStartedEvent {
            id: "utterance-start-1".to_string(),
            utterance_id: "speech-1".to_string(),
            timestamp_ms: 42,
        };
        let value = serde_json::to_value(event).unwrap_or_else(|serialization_error| {
            serde_json::json!({ "serializationError": serialization_error.to_string() })
        });

        assert_eq!(value["utteranceId"], "speech-1");
        assert_eq!(value["timestampMs"], 42);
    }

    #[test]
    fn utterance_ended_payload_uses_stable_wire_format() {
        let event = UtteranceEndedEvent {
            id: "utterance-end-1".to_string(),
            utterance_id: "speech-1".to_string(),
            reason: UtteranceEndReason::NoSpeech,
            timestamp_ms: 42,
        };
        let value = serde_json::to_value(event).unwrap_or_else(|serialization_error| {
            serde_json::json!({ "serializationError": serialization_error.to_string() })
        });

        assert_eq!(value["utteranceId"], "speech-1");
        assert_eq!(value["reason"], "noSpeech");

        let reasons = [
            (UtteranceEndReason::NoSpeech, "noSpeech"),
            (UtteranceEndReason::SttFailed, "sttFailed"),
            (UtteranceEndReason::Discarded, "discarded"),
        ];

        for (reason, expected) in reasons {
            let value = serde_json::to_value(reason).unwrap_or_else(|serialization_error| {
                serde_json::json!({ "serializationError": serialization_error.to_string() })
            });

            assert_eq!(value, expected);
        }
    }

    #[test]
    fn diagnostic_payload_includes_machine_readable_code() {
        let event = DiagnosticEvent {
            id: "diagnostic-1".to_string(),
            category: DiagnosticCategory::Osc,
            severity: DiagnosticSeverity::Error,
            code: "osc.send_failed",
            message: "OSC send failed".to_string(),
            detail: None,
            timestamp_ms: 42,
        };
        let value = serde_json::to_value(event).unwrap_or_else(|serialization_error| {
            serde_json::json!({ "serializationError": serialization_error.to_string() })
        });

        assert_eq!(value["category"], "osc");
        assert_eq!(value["severity"], "error");
        assert_eq!(value["code"], "osc.send_failed");
        assert_eq!(value["message"], "OSC send failed");
        assert!(value.get("detail").is_none());
    }

    #[test]
    fn error_codes_share_the_prefix_of_their_diagnostic_category() {
        let errors = [
            AppError::audio("x"),
            AppError::config("x"),
            AppError::config_io("x"),
            AppError::osc_encode("x".to_string()),
            AppError::osc_bind("x".to_string()),
            AppError::osc_send("127.0.0.1:9000", "x".to_string()),
            AppError::osc_send_incomplete("127.0.0.1:9000", 2, 1),
            AppError::runtime("x"),
            AppError::secret("x"),
            AppError::state("x"),
            AppError::stt("x"),
            AppError::wav("x"),
        ];

        for error in errors {
            let category = serde_json::to_value(DiagnosticCategory::for_error(&error))
                .unwrap_or_else(|serialization_error| {
                    serde_json::json!({ "serializationError": serialization_error.to_string() })
                });
            let prefix = format!("{}.", category.as_str().unwrap_or_default());

            assert!(
                error.code().starts_with(&prefix),
                "code `{}` should start with `{prefix}`",
                error.code()
            );
        }
    }
}
