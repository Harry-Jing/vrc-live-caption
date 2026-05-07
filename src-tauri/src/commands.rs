use crate::config::AppConfig;
use crate::error::AppResult;
use crate::events::{
    DiagnosticCategory, DiagnosticSeverity, RuntimeStatus, emit_diagnostic, emit_status,
    emit_transcript_final, emit_transcript_partial, next_transcript_id,
};
use crate::osc::{OSC_CHATBOX_INPUT_ADDRESS, OSC_TEST_MESSAGE, send_chatbox_osc};
use tauri::AppHandle;

#[tauri::command]
pub(crate) fn get_app_config() -> AppConfig {
    AppConfig::default()
}

#[tauri::command]
pub(crate) fn start_mock_runtime(app: AppHandle) -> AppResult<()> {
    tracing::info!("starting mock runtime");

    emit_status(
        &app,
        RuntimeStatus::Starting,
        Some("Starting mock runtime".to_string()),
    )?;
    emit_diagnostic(
        &app,
        DiagnosticCategory::Runtime,
        DiagnosticSeverity::Info,
        "Mock runtime started".to_string(),
        Some("Runtime foundation path is active.".to_string()),
    )?;
    emit_status(
        &app,
        RuntimeStatus::Running,
        Some("Mock runtime is running".to_string()),
    )
}

#[tauri::command]
pub(crate) fn emit_mock_transcript(app: AppHandle) -> AppResult<()> {
    let id = next_transcript_id("mock");

    tracing::info!(transcript_id = id, "emitting mock transcript");

    emit_transcript_partial(
        &app,
        id.clone(),
        "Testing live caption preview...".to_string(),
    )?;
    emit_transcript_final(
        &app,
        id,
        "Testing live caption preview from the mock runtime.".to_string(),
    )?;

    emit_diagnostic(
        &app,
        DiagnosticCategory::Stt,
        DiagnosticSeverity::Info,
        "Mock transcript emitted".to_string(),
        Some("The UI received normalized partial and final transcript events.".to_string()),
    )
}

#[tauri::command]
pub(crate) fn emit_mock_diagnostic(app: AppHandle) -> AppResult<()> {
    tracing::info!("emitting mock diagnostic");

    emit_diagnostic(
        &app,
        DiagnosticCategory::Config,
        DiagnosticSeverity::Info,
        "Config shape loaded".to_string(),
        Some("No API keys or provider secrets are stored in app config.".to_string()),
    )
}

#[tauri::command]
pub(crate) fn send_osc_test_message(app: AppHandle) -> AppResult<()> {
    let config = AppConfig::default();

    match send_chatbox_osc(&config.osc, OSC_TEST_MESSAGE) {
        Ok(result) => {
            tracing::info!(
                target = result.target,
                byte_count = result.byte_count,
                "sent OSC Chatbox test message"
            );

            emit_diagnostic(
                &app,
                DiagnosticCategory::Osc,
                DiagnosticSeverity::Info,
                "OSC Chatbox test sent".to_string(),
                Some(format!(
                    "Sent final-only test text to {} with {}.",
                    result.target, OSC_CHATBOX_INPUT_ADDRESS
                )),
            )
        }
        Err(error) => {
            tracing::warn!(
                code = error.code,
                error_message = %error.message,
                "OSC test failed"
            );
            let diagnostic_detail = error.message.clone();

            emit_diagnostic(
                &app,
                DiagnosticCategory::Osc,
                DiagnosticSeverity::Error,
                "OSC Chatbox test failed".to_string(),
                Some(diagnostic_detail),
            )?;

            Err(error)
        }
    }
}
