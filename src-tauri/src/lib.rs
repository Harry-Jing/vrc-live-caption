mod audio;
mod chatbox_layout;
mod chatbox_pacer;
mod chatbox_publisher;
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
            commands::get_app_config,
            commands::save_app_config,
            commands::list_audio_input_devices,
            commands::start_runtime,
            commands::stop_runtime,
            commands::get_runtime_status,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::error::{AppError, AppResult};
    use std::fs;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    #[expect(
        deprecated,
        reason = "Tauri's mock runtime requires one run iteration to execute the production setup hook."
    )]
    fn builder_setup_loads_saved_config_before_commands_run() -> AppResult<()> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_nanos();
        let identifier = format!(
            "com.vrclivecaption.startup-test-{}-{nonce}",
            std::process::id()
        );
        let mut probe_context = tauri::test::mock_context(tauri::test::noop_assets());
        probe_context
            .config_mut()
            .identifier
            .clone_from(&identifier);
        let probe_app = tauri::test::mock_builder()
            .build(probe_context)
            .map_err(|error| AppError::runtime(format!("Failed to build path probe: {error}")))?;
        let config_directory = probe_app.path().app_config_dir().map_err(|error| {
            AppError::config_io(format!("Failed to resolve test path: {error}"))
        })?;
        drop(probe_app);

        let mut saved_config = AppConfig::default();
        saved_config.audio.input_device_id = Some("saved-device".to_string());
        saved_config.osc.enabled = false;
        fs::create_dir_all(&config_directory).map_err(|error| {
            AppError::config_io(format!("Failed to create test config directory: {error}"))
        })?;
        let mut contents = serde_json::to_value(&saved_config).map_err(|error| {
            AppError::config_io(format!("Failed to serialize test config: {error}"))
        })?;
        contents["osc"]["minIntervalMs"] = serde_json::json!(750);
        let contents = serde_json::to_string(&contents).map_err(|error| {
            AppError::config_io(format!("Failed to serialize legacy test config: {error}"))
        })?;
        fs::write(config_directory.join("config.json"), contents).map_err(|error| {
            AppError::config_io(format!("Failed to write test config: {error}"))
        })?;

        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        context.config_mut().identifier = identifier;
        let mut app = configure_builder(tauri::test::mock_builder())
            .build(context)
            .map_err(|error| AppError::runtime(format!("Failed to build test app: {error}")))?;
        app.run_iteration(|_, _| {});
        let loaded_config = app.state::<state::AppState>().config()?;
        let persisted_contents =
            fs::read_to_string(config_directory.join("config.json")).map_err(|error| {
                AppError::config_io(format!("Failed to read legacy test config: {error}"))
            })?;
        let persisted_config = serde_json::from_str::<serde_json::Value>(&persisted_contents)
            .map_err(|error| {
                AppError::config_io(format!("Failed to parse legacy test config: {error}"))
            })?;
        drop(app);

        fs::remove_dir_all(&config_directory).map_err(|error| {
            AppError::config_io(format!("Failed to remove test config directory: {error}"))
        })?;

        assert_eq!(
            loaded_config.audio.input_device_id.as_deref(),
            Some("saved-device")
        );
        assert!(!loaded_config.osc.enabled);
        assert_eq!(
            persisted_config.pointer("/osc/minIntervalMs"),
            Some(&serde_json::json!(750))
        );

        Ok(())
    }
}
