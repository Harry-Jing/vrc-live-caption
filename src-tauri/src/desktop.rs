//! Concrete Tauri desktop shell.
//!
//! This module owns the managed application state, startup configuration load,
//! command registration, command adapters, and process-exit cleanup. The crate
//! root installs this shell without learning its handlers or state layout.
//! Handlers stay asynchronous because their file, credential, audio, network,
//! and runtime-join work must never block Tauri's main thread.

mod state;

use crate::audio::{AudioInputDevice, AudioProbeRequest, AudioProbeResult, list_input_devices};
use crate::caption::CaptionAggregateSnapshot;
use crate::chatbox::OSC_CHATBOX_INPUT_ADDRESS;
use crate::config::AppConfig;
use crate::credentials::CredentialId;
use crate::error::AppResult;
use crate::events::{
    DiagnosticCategory, DiagnosticUpdate, emit_diagnostic, emit_runtime_control_changed,
};
use crate::runtime_control::RuntimeControlSnapshot;
use state::AppState;
use tauri::{AppHandle, Manager, Runtime, State};

// Tauri's command wrappers are Wry-bound, while setup tests use MockRuntime.
// Keep only the managed-state portion generic so tests execute the production
// setup hook without widening the desktop facade's production interface.
fn manage_state<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.manage(AppState::default()).setup(|app| {
        app.state::<AppState>().load_config(app.handle())?;
        Ok(())
    })
}

pub(super) fn install(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    manage_state(builder).invoke_handler(tauri::generate_handler![
        save_app_config,
        list_audio_input_devices,
        probe_audio_input,
        start_runtime,
        stop_runtime,
        get_runtime_control_snapshot,
        get_caption_aggregate_snapshot,
        send_osc_test_message,
        save_credential,
        delete_credential
    ])
}

pub(super) fn handle_run_event<R: Runtime>(app: &AppHandle<R>, event: tauri::RunEvent) {
    // Stop explicitly so the microphone is released and the recognition owner joins
    // before the process dies. Runtime correctness never depends on an event
    // emit reaching a webview that is already being torn down.
    if matches!(event, tauri::RunEvent::Exit)
        && let Err(error) = app.state::<AppState>().stop_runtime(app)
    {
        tracing::warn!(error_message = %error, "failed to stop runtime on exit");
    }
}

#[tauri::command(async)]
fn save_app_config(
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
fn list_audio_input_devices(app: AppHandle) -> AppResult<Vec<AudioInputDevice>> {
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
fn probe_audio_input(
    app: AppHandle,
    state: State<'_, AppState>,
    request: AudioProbeRequest,
) -> AppResult<AudioProbeResult> {
    state.probe_audio_input(&app, &request)
}

#[tauri::command(async)]
fn start_runtime(app: AppHandle, state: State<'_, AppState>) -> AppResult<RuntimeControlSnapshot> {
    tracing::info!("starting caption runtime");
    let snapshot = state.start_runtime(&app)?;
    emit_runtime_control_changed(&app, snapshot.clone());
    Ok(snapshot)
}

#[tauri::command(async)]
fn stop_runtime(app: AppHandle, state: State<'_, AppState>) -> AppResult<RuntimeControlSnapshot> {
    tracing::info!("stopping caption runtime");
    let snapshot = state.stop_runtime(&app)?;
    emit_runtime_control_changed(&app, snapshot.clone());
    Ok(snapshot)
}

#[tauri::command(async)]
fn get_runtime_control_snapshot(state: State<'_, AppState>) -> AppResult<RuntimeControlSnapshot> {
    state.runtime_control_snapshot()
}

#[tauri::command(async)]
fn get_caption_aggregate_snapshot(
    state: State<'_, AppState>,
) -> AppResult<CaptionAggregateSnapshot> {
    state.caption_aggregate_snapshot()
}

#[tauri::command(async)]
fn send_osc_test_message(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    match state.send_osc_test_message() {
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
fn save_credential(
    app: AppHandle,
    state: State<'_, AppState>,
    id: CredentialId,
    secret: String,
) -> AppResult<RuntimeControlSnapshot> {
    let snapshot = state.save_credential(id, secret)?;
    emit_runtime_control_changed(&app, snapshot.clone());

    emit_diagnostic(
        &app,
        DiagnosticUpdate::info(
            DiagnosticCategory::Config,
            "config.credential_saved",
            "Credential saved",
            "The API key was saved in the system credential store, not app config.",
        ),
    );

    Ok(snapshot)
}

#[tauri::command(async)]
fn delete_credential(
    app: AppHandle,
    state: State<'_, AppState>,
    id: CredentialId,
) -> AppResult<RuntimeControlSnapshot> {
    let snapshot = state.delete_credential(id)?;
    emit_runtime_control_changed(&app, snapshot.clone());

    emit_diagnostic(
        &app,
        DiagnosticUpdate::info(
            DiagnosticCategory::Config,
            "config.credential_deleted",
            "Credential removed",
            "The saved credential was removed from secure storage.",
        ),
    );

    Ok(snapshot)
}

#[cfg(test)]
#[path = "desktop_tests.rs"]
mod tests;
