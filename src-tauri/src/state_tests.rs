use super::*;
use crate::capability_planner::plan_runtime;
use crate::runtime_control::{
    RuntimeChatboxSnapshot, RuntimeSelectedConfig, RuntimeSessionPhase, RuntimeSessionSnapshot,
    RuntimeStatus, RuntimeStatusEvent,
};
use std::thread;
use std::time::Duration;
use tauri::{Listener, Manager};

fn session_snapshot(config: &AppConfig, generation: u64) -> RuntimeSessionSnapshot {
    RuntimeSessionSnapshot {
        generation,
        phase: RuntimeSessionPhase::Starting,
        started_from_config_revision: 0,
        selected: RuntimeSelectedConfig::from(config),
        runtime_plan: plan_runtime(config),
        credential: None,
        chatbox: RuntimeChatboxSnapshot::Disabled {
            host: config.osc.host.clone(),
            port: config.osc.port,
        },
        uploads_microphone_audio: false,
    }
}

#[test]
fn default_config_passes_validation() -> AppResult<()> {
    AppConfig::default().validate()
}

#[test]
fn removed_config_requires_an_explicit_review_before_start() -> AppResult<()> {
    let error = ensure_config_was_reviewed(true)
        .err()
        .ok_or_else(|| AppError::state("An unreviewed migrated config unexpectedly started."))?;

    assert_eq!(error.code(), "config.invalid");
    assert!(error.to_string().contains("Review and save"));
    ensure_config_was_reviewed(false)?;
    Ok(())
}

#[test]
fn stop_epoch_prevents_a_late_start_error_from_overwriting_stopped() -> AppResult<()> {
    let app = tauri::test::mock_builder()
        .manage(AppState::default())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .map_err(|error| AppError::runtime(format!("Failed to build test app: {error}")))?;
    let state = app.state::<AppState>();
    let expected_stop_epoch = state.runtime.stop_epoch();

    state.runtime.stop(app.handle())?;
    let recorded = state.record_start_error_if_current(
        &AppError::runtime("Late failure from a cancelled Start."),
        None,
        expected_stop_epoch,
    )?;
    let snapshot = state.runtime_control_snapshot()?;

    assert!(recorded.is_none());
    assert_eq!(snapshot.runtime.status, RuntimeStatus::Stopped);
    assert!(snapshot.session.is_none());
    Ok(())
}

#[test]
fn recorded_start_error_publishes_control_before_legacy_status() -> AppResult<()> {
    let app = tauri::test::mock_builder()
        .manage(AppState::default())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .map_err(|error| AppError::runtime(format!("Failed to build test app: {error}")))?;
    let (event_sender, event_receiver) = std::sync::mpsc::channel();
    let control_sender = event_sender.clone();
    app.listen("runtime-control-changed", move |event| {
        let _ = control_sender.send(("control", event.payload().to_string()));
    });
    app.listen("runtime-status", move |event| {
        let _ = event_sender.send(("status", event.payload().to_string()));
    });
    let snapshot = app
        .state::<AppState>()
        .record_start_error(&AppError::config("Invalid test configuration."), None)?;

    emit_runtime_control_and_status(app.handle(), snapshot);

    let (first_kind, first_payload) = event_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("Start error control event was not delivered."))?;
    let (second_kind, second_payload) = event_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("Start error status event was not delivered."))?;
    let control = serde_json::from_str::<serde_json::Value>(&first_payload).map_err(|error| {
        AppError::runtime(format!(
            "Failed to parse start error control event: {error}"
        ))
    })?;
    let status = serde_json::from_str::<serde_json::Value>(&second_payload).map_err(|error| {
        AppError::runtime(format!("Failed to parse start error status event: {error}"))
    })?;

    assert_eq!(first_kind, "control");
    assert_eq!(second_kind, "status");
    assert_eq!(control["runtime"]["status"], "error");
    assert!(control["session"].is_null());
    assert_eq!(status["status"], "error");
    Ok(())
}

#[test]
fn incompatible_publication_fails_before_openai_credentials_are_resolved() -> AppResult<()> {
    let mut config = AppConfig::default();
    config.publication.mode = crate::config::PublicationMode::Live;
    let plan = plan_runtime(&config);
    let Err(error) = ensure_runtime_plan_is_startable(&plan) else {
        return Err(AppError::state(
            "Bounded OpenAI Live unexpectedly passed runtime preflight.",
        ));
    };

    assert_eq!(error.code(), "config.invalid");
    assert!(error.to_string().contains("publication.mode_unsupported"));
    assert!(
        !error.to_string().contains("API key"),
        "planner failure must win over missing credentials"
    );
    assert_eq!(
        config.publication.mode,
        crate::config::PublicationMode::Live
    );

    Ok(())
}

#[test]
fn gpt_live_transcribe_live_publication_passes_runtime_preflight() -> AppResult<()> {
    let mut config = AppConfig::default();
    config.stt.model = crate::config::OpenAiTranscriptionModel::GptLiveTranscribe;
    config.publication.mode = crate::config::PublicationMode::Live;

    ensure_runtime_plan_is_startable(&plan_runtime(&config))
}

#[test]
fn stop_does_not_hold_the_control_lock_while_status_events_clear_session() -> AppResult<()> {
    let app = tauri::test::mock_builder()
        .manage(AppState::default())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .map_err(|error| AppError::runtime(format!("Failed to build test app: {error}")))?;
    let state = app.state::<AppState>();
    let selected = AppConfig::default();
    state
        .control
        .install_starting_session(session_snapshot(&selected, 5))?;
    state
        .runtime_status_recorder()
        .record(RuntimeStatusEvent::new(
            RuntimeStatus::Running,
            Some("running".to_string()),
        ))?;

    let stop_handle = app.handle().clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = thread::spawn(move || {
        let result = stop_handle.state::<AppState>().stop_runtime(&stop_handle);
        let _ = sender.send(result);
    });
    let snapshot = receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("Stop deadlocked while publishing status."))??;
    worker
        .join()
        .map_err(|_| AppError::runtime("Stop test thread panicked."))?;

    assert_eq!(snapshot.runtime.status, RuntimeStatus::Stopped);
    assert!(snapshot.session.is_none());
    Ok(())
}

#[test]
fn stop_is_not_blocked_by_a_desired_state_operation() -> AppResult<()> {
    let app = tauri::test::mock_builder()
        .manage(AppState::default())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .map_err(|error| AppError::runtime(format!("Failed to build test app: {error}")))?;
    let state = app.state::<AppState>();
    let blocked_operation = state
        .desired_state_gate
        .lock()
        .map_err(|_| AppError::state("Desired-state operation gate was poisoned."))?;
    let stop_handle = app.handle().clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = thread::spawn(move || {
        let result = stop_handle.state::<AppState>().stop_runtime(&stop_handle);
        let _ = sender.send(result);
    });

    let prompt_result = receiver.recv_timeout(Duration::from_millis(100));
    drop(blocked_operation);
    let (completed_promptly, stop_result) = match prompt_result {
        Ok(result) => (true, result),
        Err(_) => (
            false,
            receiver.recv_timeout(Duration::from_secs(1)).map_err(|_| {
                AppError::runtime("Stop did not finish after the desired-state gate opened.")
            })?,
        ),
    };
    worker
        .join()
        .map_err(|_| AppError::runtime("Stop priority test thread panicked."))?;
    let snapshot = stop_result?;

    assert!(
        completed_promptly,
        "Stop waited for an unrelated desired-state operation."
    );
    assert_eq!(snapshot.runtime.status, RuntimeStatus::Stopped);
    Ok(())
}
