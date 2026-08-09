use super::*;
use crate::error::{AppResult, ProviderFailureClass};
use tauri::Listener;

fn registered_tauri_commands() -> AppResult<Vec<String>> {
    let handler = include_str!("lib.rs")
        .split_once("tauri::generate_handler![")
        .and_then(|(_, remainder)| remainder.split_once(']'))
        .map(|(handler, _)| handler)
        .ok_or_else(|| AppError::state("Tauri invoke handler must remain discoverable"))?;

    Ok(handler
        .lines()
        .filter_map(|line| line.trim().strip_prefix("commands::"))
        .map(|command| command.trim_end_matches(',').to_string())
        .collect())
}

fn build_manifest_commands() -> AppResult<Vec<String>> {
    let manifest = include_str!("../build.rs")
        .split_once("const APP_COMMANDS: &[&str] = &[")
        .and_then(|(_, remainder)| remainder.split_once("];"))
        .map(|(commands, _)| commands)
        .ok_or_else(|| AppError::state("Tauri build command manifest must remain discoverable"))?;

    Ok(manifest
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix('"')
                .and_then(|command| command.strip_suffix("\","))
                .map(str::to_owned)
        })
        .collect())
}

#[test]
fn tauri_ipc_names_match_the_shared_contract() -> AppResult<()> {
    let manifest = serde_json::from_str::<serde_json::Value>(include_str!(
        "../../contracts/tauri-ipc-v1.json"
    ))
    .map_err(|error| AppError::config(format!("Failed to parse Tauri IPC contract: {error}")))?;
    let expected_events = serde_json::json!({
        "runtimeStatus": EVENT_RUNTIME_STATUS,
        "runtimeControlChanged": EVENT_RUNTIME_CONTROL_CHANGED,
        "captionSessionChanged": EVENT_CAPTION_SESSION_CHANGED,
        "audioLevel": EVENT_AUDIO_LEVEL,
        "diagnostic": EVENT_DIAGNOSTIC,
    });
    assert_eq!(manifest.get("events"), Some(&expected_events));

    let mut expected_commands = manifest
        .get("commands")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| AppError::config("Tauri IPC contract must define command names"))?
        .values()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| AppError::config("Tauri command names must be strings"))
        })
        .collect::<AppResult<Vec<_>>>()?;
    let mut registered_commands = registered_tauri_commands()?;
    let mut build_commands = build_manifest_commands()?;
    expected_commands.sort();
    registered_commands.sort();
    build_commands.sort();

    assert_eq!(registered_commands, expected_commands);
    assert_eq!(build_commands, expected_commands);
    Ok(())
}

#[test]
fn status_snapshot_is_updated_before_event_delivery() -> AppResult<()> {
    let app = tauri::test::mock_builder()
        .manage(AppState::default())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .map_err(|error| AppError::runtime(format!("Failed to build test app: {error}")))?;
    let listener_handle = app.handle().clone();
    let (snapshot_sender, snapshot_receiver) = std::sync::mpsc::channel();

    app.listen(EVENT_RUNTIME_STATUS, move |_| {
        let snapshot = listener_handle
            .state::<AppState>()
            .runtime_control_snapshot()
            .and_then(|control| {
                let status = control.runtime;
                serde_json::to_value(status).map_err(|error| {
                    AppError::runtime(format!("Failed to serialize status snapshot: {error}"))
                })
            });
        let _ = snapshot_sender.send(snapshot);
    });

    record_and_emit_runtime_status(
        app.handle(),
        RuntimeStatus::Running,
        Some("Listening for microphone speech".to_string()),
    );

    let snapshot = snapshot_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("Runtime status event was not delivered."))??;
    assert_eq!(snapshot["status"], "running");
    assert_eq!(snapshot["message"], "Listening for microphone speech");

    Ok(())
}

#[test]
fn authoritative_control_event_precedes_the_legacy_status_event() -> AppResult<()> {
    let app = tauri::test::mock_builder()
        .manage(AppState::default())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .map_err(|error| AppError::runtime(format!("Failed to build test app: {error}")))?;
    let (event_sender, event_receiver) = std::sync::mpsc::channel();
    let control_sender = event_sender.clone();
    app.listen(EVENT_RUNTIME_CONTROL_CHANGED, move |event| {
        let _ = control_sender.send(("control", event.payload().to_string()));
    });
    app.listen(EVENT_RUNTIME_STATUS, move |event| {
        let _ = event_sender.send(("status", event.payload().to_string()));
    });

    record_and_emit_runtime_status(
        app.handle(),
        RuntimeStatus::Running,
        Some("running".to_string()),
    );

    let (first_kind, first_payload) = event_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("Control event was not delivered."))?;
    let (second_kind, _) = event_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("Legacy status event was not delivered."))?;
    let control = serde_json::from_str::<serde_json::Value>(&first_payload)
        .map_err(|error| AppError::runtime(format!("Failed to parse control event: {error}")))?;

    assert_eq!(first_kind, "control");
    assert_eq!(second_kind, "status");
    assert_eq!(control["runtime"]["status"], "running");
    assert_eq!(control["revision"], 1);
    Ok(())
}

#[test]
fn utterance_started_payload_uses_stable_wire_format() {
    let event = UtteranceStartedEvent {
        id: "utterance-start-1".to_string(),
        generation: 7,
        stream_id: "recognition-7-1".to_string(),
        utterance_id: "speech-1".to_string(),
        timestamp_ms: 42,
    };
    let value = serde_json::to_value(event).unwrap_or_else(|serialization_error| {
            serde_json::json!({ "serializationError": serialization_error.to_string() })
        });

    assert_eq!(value["generation"], 7);
    assert_eq!(value["streamId"], "recognition-7-1");
    assert_eq!(value["utteranceId"], "speech-1");
    assert_eq!(value["timestampMs"], 42);
}

#[test]
fn utterance_ended_payload_uses_stable_wire_format() {
    let event = UtteranceEndedEvent {
        id: "utterance-end-1".to_string(),
        generation: 7,
        stream_id: "recognition-7-1".to_string(),
        utterance_id: "speech-1".to_string(),
        reason: UtteranceEndReason::NoSpeech,
        timestamp_ms: 42,
    };
    let value = serde_json::to_value(event).unwrap_or_else(|serialization_error| {
            serde_json::json!({ "serializationError": serialization_error.to_string() })
        });

    assert_eq!(value["generation"], 7);
    assert_eq!(value["streamId"], "recognition-7-1");
    assert_eq!(value["utteranceId"], "speech-1");
    assert_eq!(value["reason"], "noSpeech");

    let reasons = [
        (UtteranceEndReason::NoSpeech, "noSpeech"),
        (UtteranceEndReason::SttFailed, "sttFailed"),
    ];

    for (reason, expected) in reasons {
        let value = serde_json::to_value(reason).unwrap_or_else(|serialization_error| {
                serde_json::json!({ "serializationError": serialization_error.to_string() })
            });

        assert_eq!(value, expected);
    }
}

#[test]
fn diagnostic_payload_includes_machine_readable_code() {
    let event = DiagnosticEvent {
        id: "diagnostic-1".to_string(),
        category: DiagnosticCategory::Osc,
        severity: DiagnosticSeverity::Error,
        code: "osc.send_failed",
        message: "OSC send failed".to_string(),
        detail: None,
        timestamp_ms: 42,
    };
    let value = serde_json::to_value(event).unwrap_or_else(|serialization_error| {
            serde_json::json!({ "serializationError": serialization_error.to_string() })
        });

    assert_eq!(value["category"], "osc");
    assert_eq!(value["severity"], "error");
    assert_eq!(value["code"], "osc.send_failed");
    assert_eq!(value["message"], "OSC send failed");
    assert!(value.get("detail").is_none());
}

#[test]
fn audio_level_payload_contains_only_scalar_generation_telemetry() {
    let event = AudioLevelEvent {
        generation: 7,
        revision: 3,
        rms_dbfs: -24.5,
        peak_dbfs: -10.0,
        clipping: false,
        gate_open: true,
        timestamp_ms: 42,
    };
    let value = serde_json::to_value(event).unwrap_or_else(|serialization_error| {
        serde_json::json!({ "serializationError": serialization_error.to_string() })
    });

    assert_eq!(value["generation"], 7);
    assert_eq!(value["revision"], 3);
    assert_eq!(value["rmsDbfs"], -24.5);
    assert_eq!(value["peakDbfs"], -10.0);
    assert_eq!(value["clipping"], false);
    assert_eq!(value["gateOpen"], true);
    assert_eq!(value["timestampMs"], 42);
    assert!(value.get("samples").is_none());
}

#[test]
fn error_codes_share_the_prefix_of_their_diagnostic_category() {
    let errors = [
        AppError::audio("x"),
        AppError::config("x"),
        AppError::config_io("x"),
        AppError::osc_encode("x".to_string()),
        AppError::osc_bind("x".to_string()),
        AppError::osc_send("127.0.0.1:9000", "x".to_string()),
        AppError::osc_send_incomplete("127.0.0.1:9000", 2, 1),
        AppError::runtime("x"),
        AppError::secret("x"),
        AppError::state("x"),
        AppError::stt("x"),
        AppError::stt_provider(ProviderFailureClass::Unknown, "x"),
        AppError::stt_backpressure("x"),
        AppError::stt_network_terminal("x"),
    ];

    for error in errors {
        let category = serde_json::to_value(DiagnosticCategory::for_error(&error))
                .unwrap_or_else(|serialization_error| {
                    serde_json::json!({ "serializationError": serialization_error.to_string() })
                });
        let prefix = format!("{}.", category.as_str().unwrap_or_default());

        assert!(
            error.code().starts_with(&prefix),
            "code `{}` should start with `{prefix}`",
            error.code()
        );
    }
}
