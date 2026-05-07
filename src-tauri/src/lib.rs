mod commands;
mod config;
mod error;
mod events;
mod osc;
mod runtime;

pub fn run() {
    let _ = tracing_subscriber::fmt().with_target(false).try_init();

    tauri::Builder::default()
        .manage(runtime::RuntimeState::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_app_config,
            commands::start_mock_runtime,
            commands::emit_mock_transcript,
            commands::emit_mock_diagnostic,
            commands::send_osc_test_message
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
