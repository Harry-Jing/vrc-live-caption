use super::*;
use crate::capability_planner::plan_runtime;
use crate::runtime_control::{RuntimeChatboxSnapshot, RuntimeSelectedConfig};
use crate::secrets::ProviderSecretStorage;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;
use tauri::Listener;

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
fn runtime_control_snapshot_has_a_versioned_authoritative_shape() -> AppResult<()> {
    let state = AppState::default();
    let snapshot = state.runtime_control_snapshot()?;
    let value = serde_json::to_value(snapshot)
        .map_err(|error| AppError::state(format!("Failed to serialize snapshot: {error}")))?;

    assert_eq!(value["contractVersion"], serde_json::json!(3));
    assert_eq!(value["revision"], serde_json::json!(0));
    assert_eq!(value["desired"]["revision"], serde_json::json!(0));
    assert_eq!(
        value["desired"]["config"]["schemaVersion"],
        serde_json::json!(3)
    );
    assert_eq!(
        value["desired"]["runtimePlan"]["publication"]["state"],
        serde_json::json!("ready")
    );
    assert!(value["session"].is_null());
    assert_eq!(value["pendingChanges"], serde_json::json!([]));

    Ok(())
}

#[test]
fn snapshot_reads_the_cached_desired_secret_status() -> AppResult<()> {
    let state = AppState::default();
    {
        let mut control = state.lock_control()?;
        control.provider_secrets = vec![ProviderSecretStatus {
            provider: "openai".to_string(),
            configured: true,
            storage: Some(ProviderSecretStorage::Environment),
            display_suffix: Some("test".to_string()),
            error: None,
        }];
    }

    let snapshot = state.runtime_control_snapshot()?;
    assert_eq!(
        snapshot.desired.provider_secrets[0]
            .display_suffix
            .as_deref(),
        Some("test")
    );
    Ok(())
}

#[test]
fn snapshot_reads_cannot_mix_a_revision_with_another_config() -> AppResult<()> {
    let state = Arc::new(AppState::default());
    let barrier = Arc::new(Barrier::new(2));
    let writer_state = Arc::clone(&state);
    let writer_barrier = Arc::clone(&barrier);
    let writer = thread::spawn(move || -> AppResult<()> {
        writer_barrier.wait();
        for revision in 1..=2_000_u64 {
            let mut control = writer_state.lock_control()?;
            control.revision = revision;
            control.config_revision = revision;
            control.config.stt.languages = vec![format!("revision-{revision}")];
        }
        Ok(())
    });

    barrier.wait();
    for _ in 0..2_000 {
        let snapshot = state.runtime_control_snapshot()?;
        if snapshot.revision > 0 {
            assert_eq!(snapshot.desired.revision, snapshot.revision);
            assert_eq!(
                snapshot.desired.config.stt.languages,
                vec![format!("revision-{}", snapshot.revision)]
            );
        }
    }

    writer
        .join()
        .map_err(|_| AppError::runtime("Snapshot writer test thread panicked."))??;
    Ok(())
}

#[test]
fn runtime_error_preserves_the_effective_session_but_stopped_clears_it() -> AppResult<()> {
    let state = AppState::default();
    let selected = AppConfig::default();
    state.install_starting_session(session_snapshot(&selected, 7))?;

    let error_snapshot = state.record_runtime_status(RuntimeStatusEvent::new(
        RuntimeStatus::Error,
        Some("test failure".to_string()),
    ))?;
    assert_eq!(
        error_snapshot.session.as_ref().map(|session| session.phase),
        Some(RuntimeSessionPhase::Error)
    );

    let stopped_snapshot = state.record_runtime_status(RuntimeStatusEvent::new(
        RuntimeStatus::Stopped,
        Some("stopped".to_string()),
    ))?;
    assert!(stopped_snapshot.session.is_none());
    Ok(())
}

#[test]
fn failed_new_start_clears_an_old_error_session() -> AppResult<()> {
    let state = AppState::default();
    let selected = AppConfig::default();
    let mut old_session = session_snapshot(&selected, 11);
    old_session.phase = RuntimeSessionPhase::Error;
    state.install_starting_session(old_session)?;

    let snapshot =
        state.record_start_error(&AppError::secret("OpenAI API key is missing."), None)?;

    assert_eq!(snapshot.runtime.status, RuntimeStatus::Error);
    assert!(snapshot.session.is_none());
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

    emit_recorded_status(app.handle(), snapshot);

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
fn thread_spawn_failure_preserves_the_session_it_already_installed() -> AppResult<()> {
    let state = AppState::default();
    let selected = AppConfig::default();
    state.install_starting_session(session_snapshot(&selected, 12))?;

    let snapshot = state.record_start_error(
        &AppError::runtime("Runtime thread could not start."),
        Some(12),
    )?;

    assert_eq!(
        snapshot.session.as_ref().map(|session| session.phase),
        Some(RuntimeSessionPhase::Error)
    );
    Ok(())
}

#[test]
fn osc_test_keeps_using_an_error_sessions_selected_target() -> AppResult<()> {
    let state = AppState::default();
    let mut selected = AppConfig::default();
    selected.osc.host = "192.0.2.10".to_string();
    selected.osc.port = 9010;
    let mut session = session_snapshot(&selected, 4);
    session.phase = RuntimeSessionPhase::Error;
    state.install_starting_session(session)?;
    {
        let mut control = state.lock_control()?;
        control.config.osc.host = "198.51.100.20".to_string();
        control.config.osc.port = 9020;
    }

    let effective = state.osc_config_for_test()?;
    assert_eq!(effective.host, "192.0.2.10");
    assert_eq!(effective.port, 9010);
    Ok(())
}

#[test]
fn stop_does_not_hold_the_control_lock_while_status_events_clear_session() -> AppResult<()> {
    let app = tauri::test::mock_builder()
        .manage(AppState::default())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .map_err(|error| AppError::runtime(format!("Failed to build test app: {error}")))?;
    let state = app.state::<AppState>();
    let selected = AppConfig::default();
    state.install_starting_session(session_snapshot(&selected, 5))?;
    state.record_runtime_status(RuntimeStatusEvent::new(
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

#[test]
fn default_config_serializes_schema_version() -> Result<(), serde_json::Error> {
    let value = serde_json::to_value(AppConfig::default())?;

    assert_eq!(value.get("schemaVersion"), Some(&serde_json::json!(3)));
    assert_eq!(
        value.pointer("/publication/mode"),
        Some(&serde_json::json!("completed"))
    );
    assert!(value.pointer("/osc/minIntervalMs").is_none());

    Ok(())
}

#[test]
fn current_config_round_trips_without_compatibility_defaults() -> AppResult<()> {
    let mut config = AppConfig::default();
    config.audio.input_device_id = Some("saved-device".to_string());
    config.stt.languages = vec!["zh".to_string(), "en".to_string()];
    config.stt.model = crate::config::OpenAiTranscriptionModel::GptLiveTranscribe;
    config.osc.enabled = false;
    config.publication.mode = crate::config::PublicationMode::Live;
    let serialized = serde_json::to_string(&config).map_err(|error| {
        AppError::config_io(format!("Failed to serialize test config: {error}"))
    })?;
    let reparsed = parse_valid_config(&serialized)?;

    assert_eq!(reparsed, config);
    Ok(())
}

#[test]
fn current_live_publication_round_trips() -> AppResult<()> {
    let mut config = AppConfig::default();
    config.stt.model = crate::config::OpenAiTranscriptionModel::GptLiveTranscribe;
    config.publication.mode = crate::config::PublicationMode::Live;
    let serialized = serde_json::to_string(&config).map_err(|error| {
        AppError::config_io(format!("Failed to serialize test config: {error}"))
    })?;
    let reparsed = parse_valid_config(&serialized)?;

    assert_eq!(reparsed, config);
    assert_eq!(
        reparsed.publication.mode,
        crate::config::PublicationMode::Live
    );

    Ok(())
}

#[test]
fn parse_valid_config_rejects_malformed_json() {
    assert!(parse_valid_config("{ not json").is_err());
}

#[test]
fn parse_valid_config_rejects_removed_singular_language() -> AppResult<()> {
    let mut value = serde_json::to_value(AppConfig::default())
        .map_err(|error| AppError::config(format!("Failed to build test JSON: {error}")))?;
    let stt = value
        .get_mut("stt")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| AppError::state("Test config is missing stt."))?;
    stt.remove("languages");
    stt.insert("language".to_string(), serde_json::json!("en"));

    assert!(parse_valid_config(&value.to_string()).is_err());
    Ok(())
}

#[test]
fn parse_valid_config_rejects_removed_mock_provider_and_arbitrary_model() -> AppResult<()> {
    let mut value = serde_json::to_value(AppConfig::default())
        .map_err(|error| AppError::config(format!("Failed to build test JSON: {error}")))?;
    value["stt"]["provider"] = serde_json::json!("mock");
    value["stt"]["model"] = serde_json::json!("saved-model");

    assert!(parse_valid_config(&value.to_string()).is_err());
    Ok(())
}

#[test]
fn parse_valid_config_ignores_only_the_removed_osc_interval() -> AppResult<()> {
    let mut value = serde_json::to_value(AppConfig::default())
        .map_err(|error| AppError::config(format!("Failed to build test JSON: {error}")))?;
    value["osc"]["host"] = serde_json::json!("192.0.2.25");
    value["osc"]["minIntervalMs"] = serde_json::json!(750);

    let config = parse_valid_config(&value.to_string())?;
    assert_eq!(config.osc.host, "192.0.2.25");
    assert!(
        serde_json::to_value(config)
            .map_err(|error| AppError::config(format!("Failed to serialize config: {error}")))?
            .pointer("/osc/minIntervalMs")
            .is_none()
    );
    Ok(())
}

#[test]
fn parse_valid_config_still_rejects_other_unknown_fields() -> AppResult<()> {
    let mut value = serde_json::to_value(AppConfig::default())
        .map_err(|error| AppError::config(format!("Failed to build test JSON: {error}")))?;
    value["osc"]["unknownSetting"] = serde_json::json!(true);

    assert!(parse_valid_config(&value.to_string()).is_err());
    Ok(())
}

#[test]
fn parse_valid_config_rejects_old_schema_version() -> AppResult<()> {
    let mut value = serde_json::to_value(AppConfig::default())
        .map_err(|error| AppError::config(format!("Failed to build test JSON: {error}")))?;
    value["schemaVersion"] = serde_json::json!(2);

    assert!(parse_valid_config(&value.to_string()).is_err());
    Ok(())
}
