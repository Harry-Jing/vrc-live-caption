use crate::config::AppConfig;
use crate::error::AppResult;
use crate::runtime::RuntimeState;
use tauri::{AppHandle, State};

#[tauri::command]
pub(crate) fn get_app_config(runtime: State<'_, RuntimeState>) -> AppResult<AppConfig> {
    runtime.app_config()
}

#[tauri::command]
pub(crate) fn start_mock_runtime(
    app: AppHandle,
    runtime: State<'_, RuntimeState>,
) -> AppResult<()> {
    tracing::info!("starting mock runtime");

    runtime.start_mock_runtime(&app)
}

#[tauri::command]
pub(crate) fn emit_mock_transcript(
    app: AppHandle,
    runtime: State<'_, RuntimeState>,
) -> AppResult<()> {
    runtime.emit_mock_transcript(&app)
}

#[tauri::command]
pub(crate) fn emit_mock_diagnostic(
    app: AppHandle,
    runtime: State<'_, RuntimeState>,
) -> AppResult<()> {
    runtime.emit_mock_diagnostic(&app)
}

#[tauri::command]
pub(crate) fn send_osc_test_message(
    app: AppHandle,
    runtime: State<'_, RuntimeState>,
) -> AppResult<()> {
    runtime.send_osc_test_message(&app)
}
