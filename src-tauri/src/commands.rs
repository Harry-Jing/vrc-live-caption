//! Tauri commands exposed to the frontend.
//!
//! Every command is `#[tauri::command(async)]` so it runs off the main thread:
//! these handlers block on file I/O, the OS credential store, audio device
//! enumeration, UDP sends, or runtime thread joins, and a plain sync command
//! would freeze the window for that duration.

use crate::audio::{AudioInputDevice, list_input_devices};
use crate::caption_session::CaptionSessionSnapshotV1;
use crate::config::{AppConfig, SttProvider};
use crate::error::AppResult;
use crate::events::{
    DiagnosticCategory, DiagnosticUpdate, emit_diagnostic, emit_runtime_control_changed,
};
use crate::osc::{ChatboxOscSender, OSC_CHATBOX_INPUT_ADDRESS, OSC_TEST_MESSAGE};
use crate::runtime_control::RuntimeControlSnapshot;
use crate::state::AppState;
use tauri::{AppHandle, State};

#[tauri::command(async)]
pub(crate) fn save_app_config(
    app: AppHandle,
    state: State<'_, AppState>,
    config: AppConfig,
) -> AppResult<RuntimeControlSnapshot> {
    let snapshot = state.save_config(&app, config)?;
    emit_runtime_control_changed(&app, snapshot.clone());

    emit_diagnostic(
        &app,
        DiagnosticUpdate::info(
            DiagnosticCategory::Config,
            "config.saved",
            "Settings saved",
            "Only non-sensitive settings are stored in app config.",
        ),
    );

    Ok(snapshot)
}

#[tauri::command(async)]
pub(crate) fn list_audio_input_devices(app: AppHandle) -> AppResult<Vec<AudioInputDevice>> {
    let devices = list_input_devices()?;

    emit_diagnostic(
        &app,
        DiagnosticUpdate::info(
            DiagnosticCategory::Audio,
            "audio.devices_refreshed",
            "Audio input devices refreshed",
            format!("Found {} microphone input device(s).", devices.len()),
        ),
    );

    Ok(devices)
}

#[tauri::command(async)]
pub(crate) fn start_runtime(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<RuntimeControlSnapshot> {
    tracing::info!("starting outgoing caption runtime");
    let snapshot = state.start_runtime(&app)?;
    emit_runtime_control_changed(&app, snapshot.clone());
    Ok(snapshot)
}

#[tauri::command(async)]
pub(crate) fn stop_runtime(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<RuntimeControlSnapshot> {
    tracing::info!("stopping outgoing caption runtime");
    let snapshot = state.stop_runtime(&app)?;
    emit_runtime_control_changed(&app, snapshot.clone());
    Ok(snapshot)
}

#[tauri::command(async)]
pub(crate) fn get_runtime_control_snapshot(
    state: State<'_, AppState>,
) -> AppResult<RuntimeControlSnapshot> {
    state.runtime_control_snapshot()
}

#[tauri::command(async)]
pub(crate) fn get_caption_session_snapshot(
    state: State<'_, AppState>,
) -> AppResult<CaptionSessionSnapshotV1> {
    state.caption_session_snapshot()
}

#[tauri::command(async)]
pub(crate) fn emit_mock_transcript(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    state.with_running_mock_session(|session| {
        tracing::info!(generation = session.generation, "emitting mock transcript");
        state.runtime.emit_mock_transcript(
            &app,
            &session.selected.stt.language,
            &session.selected.stt.model,
        )?;

        emit_diagnostic(
            &app,
            DiagnosticUpdate::info(
                DiagnosticCategory::Stt,
                "stt.mock_transcript_emitted",
                "Mock transcript emitted",
                "The UI received normalized caption-session updates from the selected Mock adapter.",
            ),
        );

        Ok(())
    })
}

#[tauri::command(async)]
pub(crate) fn send_osc_test_message(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let osc_config = state.osc_config_for_test()?;
    let chatbox_pacer = state.chatbox_pacer();

    match ChatboxOscSender::new(&osc_config).and_then(|sender| {
        chatbox_pacer
            .wait_for_turn(None)?
            .ok_or_else(|| crate::error::AppError::state("OSC Test pacing was cancelled."))?
            .attempt(|| sender.send_text(OSC_TEST_MESSAGE))
    }) {
        Ok(result) => {
            tracing::info!(
                target = result.target,
                byte_count = result.byte_count,
                "sent OSC Chatbox test message"
            );

            emit_diagnostic(
                &app,
                DiagnosticUpdate::info(
                    DiagnosticCategory::Osc,
                    "osc.test_sent",
                    "OSC Chatbox test sent",
                    format!(
                        "Sent final-only test text to {} with {}.",
                        result.target, OSC_CHATBOX_INPUT_ADDRESS
                    ),
                ),
            );

            Ok(())
        }
        Err(error) => {
            tracing::warn!(
                code = error.code(),
                error_message = %error,
                "OSC test failed"
            );

            emit_diagnostic(
                &app,
                DiagnosticUpdate::from_error(&error, "OSC Chatbox test failed"),
            );

            Err(error)
        }
    }
}

#[tauri::command(async)]
pub(crate) fn save_provider_secret(
    app: AppHandle,
    state: State<'_, AppState>,
    provider: SttProvider,
    secret: String,
) -> AppResult<RuntimeControlSnapshot> {
    let snapshot = state.save_provider_secret(provider, secret)?;
    emit_runtime_control_changed(&app, snapshot.clone());

    emit_diagnostic(
        &app,
        DiagnosticUpdate::info(
            DiagnosticCategory::Config,
            "config.provider_secret_saved",
            "Provider API key saved",
            "The API key was saved in the system credential store, not app config.",
        ),
    );

    Ok(snapshot)
}

#[tauri::command(async)]
pub(crate) fn delete_provider_secret(
    app: AppHandle,
    state: State<'_, AppState>,
    provider: SttProvider,
) -> AppResult<RuntimeControlSnapshot> {
    let snapshot = state.delete_provider_secret(provider)?;
    emit_runtime_control_changed(&app, snapshot.clone());

    emit_diagnostic(
        &app,
        DiagnosticUpdate::info(
            DiagnosticCategory::Config,
            "config.provider_secret_deleted",
            "Provider API key removed",
            "The saved provider API key was removed from secure storage.",
        ),
    );

    Ok(snapshot)
}
