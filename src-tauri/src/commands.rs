use crate::audio::{AudioInputDevice, list_input_devices};
use crate::config::AppConfig;
use crate::error::AppResult;
use crate::events::{
    DiagnosticCategory, DiagnosticSeverity, DiagnosticUpdate, RuntimeStatus, TranscriptUpdate,
    emit_diagnostic, emit_status, emit_transcript_final, emit_transcript_partial,
    next_utterance_id,
};
use crate::osc::{OSC_CHATBOX_INPUT_ADDRESS, OSC_TEST_MESSAGE, send_chatbox_osc};
use crate::state::AppState;
use tauri::{AppHandle, State};

#[tauri::command]
pub(crate) fn get_app_config(app: AppHandle, state: State<'_, AppState>) -> AppResult<AppConfig> {
    state.load_config(&app)
}

#[tauri::command]
pub(crate) fn save_app_config(
    app: AppHandle,
    state: State<'_, AppState>,
    config: AppConfig,
) -> AppResult<AppConfig> {
    let saved_config = state.save_config(&app, config)?;

    emit_diagnostic(
        &app,
        DiagnosticUpdate {
            category: DiagnosticCategory::Config,
            severity: DiagnosticSeverity::Info,
            code: "config.saved",
            message: "Settings saved".to_string(),
            detail: Some("Only non-sensitive settings are stored in app config.".to_string()),
        },
    )?;

    Ok(saved_config)
}

#[tauri::command]
pub(crate) fn list_audio_input_devices(app: AppHandle) -> AppResult<Vec<AudioInputDevice>> {
    let devices = list_input_devices()?;

    emit_diagnostic(
        &app,
        DiagnosticUpdate {
            category: DiagnosticCategory::Audio,
            severity: DiagnosticSeverity::Info,
            code: "audio.devices_refreshed",
            message: "Audio input devices refreshed".to_string(),
            detail: Some(format!(
                "Found {} microphone input device(s).",
                devices.len()
            )),
        },
    )?;

    Ok(devices)
}

#[tauri::command]
pub(crate) fn start_runtime(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let config = state.config()?;

    tracing::info!("starting outgoing caption runtime");
    state.runtime.start(app, config)
}

#[tauri::command]
pub(crate) fn stop_runtime(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    tracing::info!("stopping outgoing caption runtime");
    state.runtime.stop(&app)
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
        DiagnosticUpdate {
            category: DiagnosticCategory::Runtime,
            severity: DiagnosticSeverity::Info,
            code: "runtime.mock_started",
            message: "Mock runtime started".to_string(),
            detail: Some("Runtime foundation path is active.".to_string()),
        },
    )?;
    emit_status(
        &app,
        RuntimeStatus::Running,
        Some("Mock runtime is running".to_string()),
    )
}

#[tauri::command]
pub(crate) fn emit_mock_transcript(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let config = state.config()?;
    let utterance_id = next_utterance_id("mock");
    let language = config.stt.language.clone();
    let provider = config.stt.provider.as_str().to_string();

    tracing::info!(utterance_id = %utterance_id, "emitting mock transcript");

    emit_transcript_partial(
        &app,
        TranscriptUpdate {
            utterance_id: utterance_id.clone(),
            text: "Testing live caption preview...".to_string(),
            language: language.clone(),
            provider: provider.clone(),
            revision: 1,
        },
    )?;
    emit_transcript_final(
        &app,
        TranscriptUpdate {
            utterance_id,
            text: "Testing live caption preview from the mock runtime.".to_string(),
            language,
            provider,
            revision: 2,
        },
    )?;

    emit_diagnostic(
        &app,
        DiagnosticUpdate {
            category: DiagnosticCategory::Stt,
            severity: DiagnosticSeverity::Info,
            code: "stt.mock_transcript_emitted",
            message: "Mock transcript emitted".to_string(),
            detail: Some(
                "The UI received normalized partial and final transcript events.".to_string(),
            ),
        },
    )
}

#[tauri::command]
pub(crate) fn emit_mock_diagnostic(app: AppHandle) -> AppResult<()> {
    tracing::info!("emitting mock diagnostic");

    emit_diagnostic(
        &app,
        DiagnosticUpdate {
            category: DiagnosticCategory::Config,
            severity: DiagnosticSeverity::Info,
            code: "config.shape_loaded",
            message: "Config shape loaded".to_string(),
            detail: Some("No API keys or provider secrets are stored in app config.".to_string()),
        },
    )
}

#[tauri::command]
pub(crate) fn send_osc_test_message(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let config = state.config()?;

    match send_chatbox_osc(&config.osc, OSC_TEST_MESSAGE) {
        Ok(result) => {
            tracing::info!(
                target = result.target,
                byte_count = result.byte_count,
                "sent OSC Chatbox test message"
            );

            emit_diagnostic(
                &app,
                DiagnosticUpdate {
                    category: DiagnosticCategory::Osc,
                    severity: DiagnosticSeverity::Info,
                    code: "osc.test_sent",
                    message: "OSC Chatbox test sent".to_string(),
                    detail: Some(format!(
                        "Sent final-only test text to {} with {}.",
                        result.target, OSC_CHATBOX_INPUT_ADDRESS
                    )),
                },
            )
        }
        Err(error) => {
            tracing::warn!(
                code = error.code(),
                error_message = %error,
                "OSC test failed"
            );

            emit_diagnostic(
                &app,
                DiagnosticUpdate {
                    category: DiagnosticCategory::Osc,
                    severity: DiagnosticSeverity::Error,
                    code: error.code(),
                    message: "OSC Chatbox test failed".to_string(),
                    detail: Some(error.to_string()),
                },
            )?;

            Err(error)
        }
    }
}
