//! Normalized runtime events emitted to the Vue frontend.
//!
//! These events are the UI-facing contract for status, transcripts, and
//! diagnostics. Provider-specific raw events should be normalized before they
//! reach this module so Vue components and output sinks do not depend on STT
//! provider protocols.

use crate::error::{AppError, AppResult};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

const EVENT_RUNTIME_STATUS: &str = "runtime-status";
const EVENT_TRANSCRIPT_PARTIAL: &str = "transcript-partial";
const EVENT_TRANSCRIPT_FINAL: &str = "transcript-final";
const EVENT_DIAGNOSTIC: &str = "diagnostic-event";

static NEXT_EVENT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeStatusEvent {
    status: RuntimeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    timestamp_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
// Keep all statuses in the UI contract even before every lifecycle branch is emitted.
#[expect(
    dead_code,
    reason = "runtime status contract includes future lifecycle states"
)]
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
    pub(crate) category: DiagnosticCategory,
    pub(crate) severity: DiagnosticSeverity,
    pub(crate) code: &'static str,
    pub(crate) message: String,
    pub(crate) detail: Option<String>,
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

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DiagnosticSeverity {
    Error,
    Info,
    Warning,
}

pub(crate) fn emit_status(
    app: &AppHandle,
    status: RuntimeStatus,
    message: Option<String>,
) -> AppResult<()> {
    app.emit(
        EVENT_RUNTIME_STATUS,
        RuntimeStatusEvent {
            status,
            message,
            timestamp_ms: now_ms(),
        },
    )
    .map_err(AppError::emit)
}

pub(crate) fn emit_transcript_partial(app: &AppHandle, update: TranscriptUpdate) -> AppResult<()> {
    emit_transcript(
        app,
        EVENT_TRANSCRIPT_PARTIAL,
        TranscriptKind::Partial,
        update,
    )
}

pub(crate) fn emit_transcript_final(app: &AppHandle, update: TranscriptUpdate) -> AppResult<()> {
    emit_transcript(app, EVENT_TRANSCRIPT_FINAL, TranscriptKind::Final, update)
}

pub(crate) fn emit_diagnostic(app: &AppHandle, update: DiagnosticUpdate) -> AppResult<()> {
    app.emit(
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
    )
    .map_err(AppError::emit)
}

pub(crate) fn next_utterance_id(prefix: &str) -> String {
    next_event_id(prefix)
}

fn emit_transcript(
    app: &AppHandle,
    event_name: &str,
    kind: TranscriptKind,
    update: TranscriptUpdate,
) -> AppResult<()> {
    app.emit(
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
    )
    .map_err(AppError::emit)
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
    fn diagnostic_payload_includes_machine_readable_code() {
        let event = DiagnosticEvent {
            id: "diagnostic-1".to_string(),
            category: DiagnosticCategory::Osc,
            severity: DiagnosticSeverity::Error,
            code: "osc_send_failed",
            message: "OSC send failed".to_string(),
            detail: None,
            timestamp_ms: 42,
        };
        let value = serde_json::to_value(event).unwrap_or_else(|serialization_error| {
            serde_json::json!({ "serializationError": serialization_error.to_string() })
        });

        assert_eq!(value["category"], "osc");
        assert_eq!(value["severity"], "error");
        assert_eq!(value["code"], "osc_send_failed");
        assert_eq!(value["message"], "OSC send failed");
        assert!(value.get("detail").is_none());
    }
}
