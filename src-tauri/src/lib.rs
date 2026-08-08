mod audio;
mod audio_level;
mod audio_probe;
mod capability_planner;
mod caption_session;
mod chatbox_diagnostics;
mod chatbox_layout;
mod chatbox_pacer;
mod chatbox_publication;
mod chatbox_publisher;
mod chatbox_publisher_common;
mod chatbox_transport;
mod commands;
mod config;
mod error;
mod events;
mod host_resolver;
mod live_chatbox_publisher;
mod openai_realtime;
mod openai_realtime_transport;
mod osc;
mod recognition;
mod recognition_audio;
#[cfg(test)]
mod recognition_fakes;
mod reconnect;
mod runtime;
mod runtime_control;
mod runtime_generation;
mod secrets;
mod segmenter;
mod state;

use tauri::Manager;

fn configure_builder<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.manage(state::AppState::default()).setup(|app| {
        app.state::<state::AppState>().load_config(app.handle())?;
        Ok(())
    })
}

#[expect(
    clippy::expect_used,
    reason = "Tauri startup failure is unrecoverable and should include the canonical startup context."
)]
pub fn run() {
    let _ = tracing_subscriber::fmt().with_target(false).try_init();

    configure_builder(tauri::Builder::default())
        .invoke_handler(tauri::generate_handler![
            commands::save_app_config,
            commands::list_audio_input_devices,
            commands::probe_audio_input,
            commands::start_runtime,
            commands::stop_runtime,
            commands::get_runtime_control_snapshot,
            commands::get_caption_session_snapshot,
            commands::send_osc_test_message,
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
                && let Err(error) = app.state::<state::AppState>().stop_runtime(app)
            {
                tracing::warn!(error_message = %error, "failed to stop runtime on exit");
            }
        });
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
