mod audio;
mod commands;
mod config;
mod error;
mod events;
mod osc;
mod runtime;
mod secrets;
mod segmenter;
mod state;
mod stt;

use tauri::Manager;

#[expect(
    clippy::expect_used,
    reason = "Tauri startup failure is unrecoverable and should include the canonical startup context."
)]
pub fn run() {
    let _ = tracing_subscriber::fmt().with_target(false).try_init();

    tauri::Builder::default()
        .manage(state::AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_app_config,
            commands::save_app_config,
            commands::list_audio_input_devices,
            commands::start_runtime,
            commands::stop_runtime,
            commands::emit_mock_transcript,
            commands::send_osc_test_message,
            commands::get_provider_secret_status,
            commands::save_provider_secret,
            commands::delete_provider_secret
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Stop the runtime explicitly on exit so the microphone is
            // released and the STT worker joins before the process dies. The
            // runtime must never rely on event-emit failures to learn that
            // the app is closing.
            if matches!(event, tauri::RunEvent::Exit)
                && let Err(error) = app.state::<state::AppState>().runtime.stop(app)
            {
                tracing::warn!(error_message = %error, "failed to stop runtime on exit");
            }
        });
}
