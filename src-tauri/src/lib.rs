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
            commands::start_mock_runtime,
            commands::emit_mock_transcript,
            commands::emit_mock_diagnostic,
            commands::send_osc_test_message,
            commands::get_provider_secret_status,
            commands::save_provider_secret,
            commands::delete_provider_secret
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
