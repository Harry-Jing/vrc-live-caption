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

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub(crate) enum RuntimeStatus {
    Idle,
    Starting,
    Running,
    Stopped,
    Error,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TranscriptEvent {
    id: String,
    text: String,
    timestamp_ms: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticEvent {
    id: String,
    category: DiagnosticCategory,
    severity: DiagnosticSeverity,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    timestamp_ms: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub(crate) enum DiagnosticCategory {
    Audio,
    Config,
    Osc,
    Runtime,
    Stt,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
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

pub(crate) fn emit_transcript_partial(app: &AppHandle, id: String, text: String) -> AppResult<()> {
    emit_transcript(app, EVENT_TRANSCRIPT_PARTIAL, id, text)
}

pub(crate) fn emit_transcript_final(app: &AppHandle, id: String, text: String) -> AppResult<()> {
    emit_transcript(app, EVENT_TRANSCRIPT_FINAL, id, text)
}

pub(crate) fn emit_diagnostic(
    app: &AppHandle,
    category: DiagnosticCategory,
    severity: DiagnosticSeverity,
    message: String,
    detail: Option<String>,
) -> AppResult<()> {
    app.emit(
        EVENT_DIAGNOSTIC,
        DiagnosticEvent {
            id: next_event_id("diagnostic"),
            category,
            severity,
            message,
            detail,
            timestamp_ms: now_ms(),
        },
    )
    .map_err(AppError::emit)
}

pub(crate) fn next_transcript_id(prefix: &str) -> String {
    next_event_id(prefix)
}

fn emit_transcript(app: &AppHandle, event_name: &str, id: String, text: String) -> AppResult<()> {
    app.emit(
        event_name,
        TranscriptEvent {
            id,
            text,
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
