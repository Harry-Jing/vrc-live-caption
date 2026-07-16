//! Tauri commands exposed to the frontend.
//!
//! Every command is `#[tauri::command(async)]` so it runs off the main thread:
//! these handlers block on file I/O, the OS credential store, audio device
//! enumeration, UDP sends, or runtime thread joins, and a plain sync command
//! would freeze the window for that duration.

use crate::audio::{AudioInputDevice, list_input_devices};
use crate::config::{AppConfig, SttProvider};
use crate::error::AppResult;
use crate::events::{
    DiagnosticCategory, DiagnosticUpdate, RuntimeStatusEvent, TranscriptUpdate, emit_diagnostic,
    emit_transcript_final, emit_transcript_partial, emit_utterance_started, next_utterance_id,
};
use crate::osc::{ChatboxOscSender, OSC_CHATBOX_INPUT_ADDRESS, OSC_TEST_MESSAGE};
use crate::secrets::{
    ProviderSecretStatus, delete_provider_secret as delete_secret, provider_secret_status,
    save_provider_secret as save_secret,
};
use crate::state::AppState;
use tauri::{AppHandle, State};

#[tauri::command(async)]
pub(crate) fn get_app_config(app: AppHandle, state: State<'_, AppState>) -> AppResult<AppConfig> {
    state.load_config(&app)
}

#[tauri::command(async)]
pub(crate) fn save_app_config(
    app: AppHandle,
    state: State<'_, AppState>,
    config: AppConfig,
) -> AppResult<AppConfig> {
    let saved_config = state.save_config(&app, config)?;

    emit_diagnostic(
        &app,
        DiagnosticUpdate::info(
            DiagnosticCategory::Config,
            "config.saved",
            "Settings saved",
            "Only non-sensitive settings are stored in app config.",
        ),
    );

    Ok(saved_config)
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
pub(crate) fn start_runtime(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let config = state.config()?;
    let chatbox_pacer = state.chatbox_pacer();

    tracing::info!("starting outgoing caption runtime");
    state.runtime.start(app, config, chatbox_pacer)
}

#[tauri::command(async)]
pub(crate) fn stop_runtime(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    tracing::info!("stopping outgoing caption runtime");
    state.runtime.stop(&app)
}

#[tauri::command(async)]
pub(crate) fn get_runtime_status(state: State<'_, AppState>) -> AppResult<RuntimeStatusEvent> {
    state.runtime.status_snapshot()
}

#[tauri::command(async)]
pub(crate) fn emit_mock_transcript(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let config = state.config()?;
    let utterance_id = next_utterance_id("mock");
    let language = config.stt.language.clone();
    let provider = config.stt.provider.as_str().to_string();

    tracing::info!(utterance_id = %utterance_id, "emitting mock transcript");

    emit_utterance_started(&app, utterance_id.clone());
    emit_transcript_partial(
        &app,
        TranscriptUpdate {
            utterance_id: utterance_id.clone(),
            text: "Testing live caption preview...".to_string(),
            language: language.clone(),
            provider: provider.clone(),
            revision: 1,
        },
    );
    emit_transcript_final(
        &app,
        TranscriptUpdate {
            utterance_id,
            text: "Testing live caption preview from the mock runtime.".to_string(),
            language,
            provider,
            revision: 2,
        },
    );

    emit_diagnostic(
        &app,
        DiagnosticUpdate::info(
            DiagnosticCategory::Stt,
            "stt.mock_transcript_emitted",
            "Mock transcript emitted",
            "The UI received normalized partial and final transcript events.",
        ),
    );

    Ok(())
}

#[tauri::command(async)]
pub(crate) fn send_osc_test_message(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let config = state.config()?;
    let chatbox_pacer = state.chatbox_pacer();

    match ChatboxOscSender::new(&config.osc).and_then(|sender| {
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
pub(crate) fn get_provider_secret_status(provider: SttProvider) -> AppResult<ProviderSecretStatus> {
    Ok(provider_secret_status(provider))
}

#[tauri::command(async)]
pub(crate) fn save_provider_secret(
    app: AppHandle,
    provider: SttProvider,
    secret: String,
) -> AppResult<ProviderSecretStatus> {
    save_secret(provider, secret)?;

    emit_diagnostic(
        &app,
        DiagnosticUpdate::info(
            DiagnosticCategory::Config,
            "config.provider_secret_saved",
            "Provider API key saved",
            "The API key was saved in the system credential store, not app config.",
        ),
    );

    Ok(provider_secret_status(provider))
}

#[tauri::command(async)]
pub(crate) fn delete_provider_secret(
    app: AppHandle,
    provider: SttProvider,
) -> AppResult<ProviderSecretStatus> {
    delete_secret(provider)?;

    emit_diagnostic(
        &app,
        DiagnosticUpdate::info(
            DiagnosticCategory::Config,
            "config.provider_secret_deleted",
            "Provider API key removed",
            "The saved provider API key was removed from secure storage.",
        ),
    );

    Ok(provider_secret_status(provider))
}
