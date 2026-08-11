use super::*;
use crate::caption_pipeline::plan_caption_pipeline;
use crate::runtime_control::{
    ChatboxPublicationSnapshot, RuntimeGenerationPhase, RuntimeGenerationSelection,
    RuntimeGenerationSnapshot, RuntimeStatus, RuntimeStatusEvent,
};
use std::thread;
use std::time::Duration;
use tauri::{Listener, Manager};

fn generation_snapshot(config: &AppConfig, generation: u64) -> RuntimeGenerationSnapshot {
    RuntimeGenerationSnapshot {
        id: generation,
        phase: RuntimeGenerationPhase::Starting,
        started_from_config_revision: 0,
        selection: RuntimeGenerationSelection::from(config),
        caption_pipeline_plan: plan_caption_pipeline(config),
        credential: None,
        chatbox_publication: ChatboxPublicationSnapshot::Disabled {
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
fn audio_probe_lease_remains_active_while_failure_diagnostic_is_emitted() -> AppResult<()> {
    let app = tauri::test::mock_builder()
        .manage(AppState::default())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .map_err(|error| AppError::runtime(format!("Failed to build test app: {error}")))?;
    let listener_handle = app.handle().clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    app.listen("diagnostic-event", move |event| {
        let state = listener_handle.state::<AppState>();
        let lease_was_still_active = state.runtime.begin_audio_probe(&listener_handle).is_err();
        let _ = sender.send((event.payload().to_string(), lease_was_still_active));
    });

    let request = AudioProbeRequest {
        input_device_id: None,
        duration_ms: 0,
    };
    let Err(error) = app
        .state::<AppState>()
        .probe_audio_input(app.handle(), &request)
    else {
        return Err(AppError::state(
            "Invalid microphone probe duration unexpectedly succeeded.",
        ));
    };
    let (payload, lease_was_still_active) = receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("Microphone probe diagnostic was not delivered."))?;
    let diagnostic = serde_json::from_str::<serde_json::Value>(&payload).map_err(|error| {
        AppError::runtime(format!(
            "Failed to parse microphone probe diagnostic: {error}"
        ))
    })?;

    assert_eq!(error.code(), "audio.failed");
    assert_eq!(diagnostic["code"], "audio.failed");
    assert!(
        lease_was_still_active,
        "microphone probe lease was released before its failure diagnostic"
    );

    let state = app.state::<AppState>();
    let released_lease = state.runtime.begin_audio_probe(app.handle())?;
    drop(released_lease);
    Ok(())
}

#[test]
fn rejected_audio_probe_lease_does_not_emit_a_failure_diagnostic() -> AppResult<()> {
    let app = tauri::test::mock_builder()
        .manage(AppState::default())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .map_err(|error| AppError::runtime(format!("Failed to build test app: {error}")))?;
    let state = app.state::<AppState>();
    let active_lease = state.runtime.begin_audio_probe(app.handle())?;
    let (sender, receiver) = std::sync::mpsc::channel();
    app.listen("diagnostic-event", move |event| {
        let _ = sender.send(event.payload().to_string());
    });

    let request = AudioProbeRequest {
        input_device_id: None,
        duration_ms: 0,
    };
    let Err(error) = state.probe_audio_input(app.handle(), &request) else {
        return Err(AppError::state(
            "Concurrent microphone probe unexpectedly acquired a second lease.",
        ));
    };

    assert_eq!(error.code(), "runtime.failed");
    assert!(error.to_string().contains("already running"));
    assert!(
        receiver.recv_timeout(Duration::from_millis(100)).is_err(),
        "lease rejection emitted a persistent microphone failure diagnostic"
    );
    drop(active_lease);
    Ok(())
}

#[test]
fn unsupported_saved_config_requires_an_explicit_review_before_start() -> AppResult<()> {
    let error = ensure_config_was_reviewed(true)
        .err()
        .ok_or_else(|| AppError::state("An unreviewed saved config unexpectedly started."))?;

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

    state.stop_runtime(app.handle())?;
    let recorded = state.record_start_error_if_current(
        &AppError::runtime("Late failure from a cancelled Start."),
        None,
        expected_stop_epoch,
    )?;
    let snapshot = state.runtime_control_snapshot()?;

    assert!(recorded.is_none());
    assert_eq!(snapshot.runtime_status.status, RuntimeStatus::Stopped);
    assert!(snapshot.generation.is_none());
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
    assert_eq!(control["runtimeStatus"]["status"], "error");
    assert!(control["generation"].is_null());
    assert_eq!(status["status"], "error");
    Ok(())
}

#[test]
fn incompatible_publication_fails_before_openai_credentials_are_resolved() -> AppResult<()> {
    let mut config = AppConfig::default();
    config.publication.mode = crate::config::PublicationMode::Live;
    let plan = plan_caption_pipeline(&config);
    let Err(error) = publication_timing_for_start(&plan) else {
        return Err(AppError::state(
            "Bounded OpenAI Live unexpectedly passed runtime preflight.",
        ));
    };

    assert_eq!(error.code(), "config.invalid");
    assert!(error.to_string().contains("publication.mode_unsupported"));
    assert!(
        !error.to_string().contains("API key"),
        "Caption Pipeline Plan failure must win over missing credentials"
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
    config.recognition.path = crate::config::RecognitionPath::OpenAiGptLiveTranscribe;
    config.publication.mode = crate::config::PublicationMode::Live;

    publication_timing_for_start(&plan_caption_pipeline(&config)).map(|_| ())
}

#[test]
fn stop_does_not_hold_the_control_lock_while_status_events_clear_generation() -> AppResult<()> {
    let app = tauri::test::mock_builder()
        .manage(AppState::default())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .map_err(|error| AppError::runtime(format!("Failed to build test app: {error}")))?;
    let state = app.state::<AppState>();
    let selected = AppConfig::default();
    state
        .control
        .install_starting_generation(generation_snapshot(&selected, 5))?;
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

    assert_eq!(snapshot.runtime_status.status, RuntimeStatus::Stopped);
    assert!(snapshot.generation.is_none());
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
    let (worker_ready_sender, worker_ready_receiver) = std::sync::mpsc::sync_channel(0);
    let (begin_stop_sender, begin_stop_receiver) = std::sync::mpsc::sync_channel(0);
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = thread::spawn(move || {
        let _ = worker_ready_sender.send(());
        let _ = begin_stop_receiver.recv();
        let result = stop_handle.state::<AppState>().stop_runtime(&stop_handle);
        let _ = sender.send(result);
    });
    worker_ready_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("Stop priority test worker did not become ready."))?;
    begin_stop_sender
        .send(())
        .map_err(|_| AppError::runtime("Stop priority test worker exited before starting."))?;

    // Timing is only a deadlock guard after the worker rendezvous. Correctness
    // comes from Stop finishing while the unrelated desired-state gate is held.
    let prompt_result = receiver.recv_timeout(Duration::from_secs(2));
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
    assert_eq!(snapshot.runtime_status.status, RuntimeStatus::Stopped);
    Ok(())
}
