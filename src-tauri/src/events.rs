//! Normalized runtime events emitted to the Vue frontend.
//!
//! These events are the UI-facing contract for status, caption sessions, and
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

use crate::caption_session::CaptionSessionSnapshotV1;
use crate::error::AppError;
use crate::runtime_control::RuntimeControlSnapshot;
use crate::state::AppState;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, Runtime};

const EVENT_RUNTIME_STATUS: &str = "runtime-status";
const EVENT_RUNTIME_CONTROL_CHANGED: &str = "runtime-control-changed";
const EVENT_CAPTION_SESSION_CHANGED: &str = "caption-session-changed";
const EVENT_UTTERANCE_STARTED: &str = "utterance-started";
const EVENT_UTTERANCE_ENDED: &str = "utterance-ended";
const EVENT_AUDIO_LEVEL: &str = "audio-level";
const EVENT_DIAGNOSTIC: &str = "diagnostic-event";

static NEXT_EVENT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Serialize)]
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

    pub(crate) fn new(status: RuntimeStatus, message: Option<String>) -> Self {
        Self {
            status,
            message,
            timestamp_ms: now_ms(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RuntimeStatus {
    Idle,
    Starting,
    Running,
    Reconnecting,
    Stopping,
    Stopped,
    Error,
}

/// Start of a confirmed utterance. Emitted before any caption text exists, so
/// caption snapshots never carry placeholder text such as a listening
/// indicator.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UtteranceStartedEvent {
    id: String,
    generation: u64,
    stream_id: String,
    utterance_id: String,
    timestamp_ms: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UtteranceEndedEvent {
    id: String,
    generation: u64,
    stream_id: String,
    utterance_id: String,
    reason: UtteranceEndReason,
    timestamp_ms: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AudioLevelEvent {
    pub(crate) generation: u64,
    pub(crate) revision: u64,
    pub(crate) rms_dbfs: f32,
    pub(crate) peak_dbfs: f32,
    pub(crate) clipping: bool,
    pub(crate) gate_open: bool,
    pub(crate) timestamp_ms: u64,
}

/// Why an utterance terminated without a completed caption. Successful
/// utterances resolve through the caption-session aggregate instead.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum UtteranceEndReason {
    /// STT finished but recognized no words.
    NoSpeech,
    /// The STT request failed; details arrive as a diagnostic event.
    SttFailed,
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
            AppError::Stt { .. }
            | AppError::SttProvider { .. }
            | AppError::SttBackpressure { .. }
            | AppError::SttNetwork { .. } => Self::Stt,
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
    let snapshot = match app.try_state::<AppState>() {
        Some(state) => match state.record_runtime_status(event.clone()) {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                tracing::warn!(
                    code = error.code(),
                    error_message = %error,
                    "failed to update authoritative runtime control status"
                );
                None
            }
        },
        None => {
            tracing::warn!("runtime status emitted before app state was managed");
            None
        }
    };

    if let Some(snapshot) = snapshot {
        emit_recorded_status(app, snapshot);
    } else {
        emit_event(app, EVENT_RUNTIME_STATUS, event);
    }
}

pub(crate) fn emit_runtime_control_changed<R: Runtime>(
    app: &AppHandle<R>,
    snapshot: RuntimeControlSnapshot,
) {
    emit_event(app, EVENT_RUNTIME_CONTROL_CHANGED, snapshot);
}

pub(crate) fn emit_caption_session_changed<R: Runtime>(
    app: &AppHandle<R>,
    snapshot: CaptionSessionSnapshotV1,
) {
    emit_event(app, EVENT_CAPTION_SESSION_CHANGED, snapshot);
}

pub(crate) fn emit_recorded_status<R: Runtime>(
    app: &AppHandle<R>,
    snapshot: RuntimeControlSnapshot,
) {
    let event = snapshot.runtime.clone();
    emit_runtime_control_changed(app, snapshot);
    emit_event(app, EVENT_RUNTIME_STATUS, event);
}

pub(crate) fn emit_utterance_started<R: Runtime>(
    app: &AppHandle<R>,
    generation: u64,
    stream_id: String,
    utterance_id: String,
    timestamp_ms: u64,
) {
    emit_event(
        app,
        EVENT_UTTERANCE_STARTED,
        UtteranceStartedEvent {
            id: next_event_id("utterance-start"),
            generation,
            stream_id,
            utterance_id,
            timestamp_ms,
        },
    );
}

pub(crate) fn emit_utterance_ended<R: Runtime>(
    app: &AppHandle<R>,
    generation: u64,
    stream_id: String,
    utterance_id: String,
    reason: UtteranceEndReason,
    timestamp_ms: u64,
) {
    emit_event(
        app,
        EVENT_UTTERANCE_ENDED,
        UtteranceEndedEvent {
            id: next_event_id("utterance-end"),
            generation,
            stream_id,
            utterance_id,
            reason,
            timestamp_ms,
        },
    );
}

pub(crate) fn emit_audio_level<R: Runtime>(app: &AppHandle<R>, event: AudioLevelEvent) {
    emit_event(app, EVENT_AUDIO_LEVEL, event);
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

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
