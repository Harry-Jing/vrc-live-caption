use super::*;
use crate::error::{AppResult, ProviderFailureClass};
use crate::runtime_control::RuntimeControlStore;
use std::time::Duration;
use tauri::Listener;

fn declared_ui_facing_events() -> AppResult<Vec<String>> {
    include_str!("events.rs")
        .lines()
        .filter(|line| line.trim_start().starts_with("const EVENT_"))
        .map(|line| {
            line.trim()
                .split_once(": &str = \"")
                .and_then(|(_, value)| value.strip_suffix("\";"))
                .map(str::to_owned)
                .ok_or_else(|| AppError::state("Tauri event declaration must remain discoverable"))
        })
        .collect()
}

#[test]
fn tauri_event_names_match_the_shared_contract() -> AppResult<()> {
    let manifest_json = include_str!("../../contracts/tauri-ipc.json");
    let manifest = serde_json::from_str::<serde_json::Value>(manifest_json).map_err(|error| {
        AppError::config(format!("Failed to parse Tauri IPC contract: {error}"))
    })?;
    let expected_events = serde_json::json!({
        "runtimeStatus": EVENT_RUNTIME_STATUS,
        "runtimeControlChanged": EVENT_RUNTIME_CONTROL_CHANGED,
        "captionAggregateChanged": EVENT_CAPTION_AGGREGATE_CHANGED,
        "audioLevel": EVENT_AUDIO_LEVEL,
        "diagnostic": EVENT_DIAGNOSTIC,
    });
    assert_eq!(manifest.get("events"), Some(&expected_events));

    let mut expected_event_names = manifest
        .get("events")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| AppError::config("Tauri IPC contract must define event names"))?
        .values()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| AppError::config("Tauri event names must be strings"))
        })
        .collect::<AppResult<Vec<_>>>()?;
    let mut declared_event_names = declared_ui_facing_events()?;
    expected_event_names.sort();
    declared_event_names.sort();
    assert_eq!(declared_event_names, expected_event_names);

    Ok(())
}

#[test]
fn status_snapshot_is_updated_before_event_delivery() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let control = RuntimeControlStore::default();
    let recorder = control.status_recorder();
    let listener_control = control.clone();
    let (snapshot_sender, snapshot_receiver) = std::sync::mpsc::channel();

    app.listen(EVENT_RUNTIME_STATUS, move |_| {
        let snapshot = listener_control.snapshot().and_then(|control| {
            let status = control.runtime_status;
            serde_json::to_value(status).map_err(|error| {
                AppError::runtime(format!("Failed to serialize status snapshot: {error}"))
            })
        });
        let _ = snapshot_sender.send(snapshot);
    });

    record_and_emit_runtime_status(
        app.handle(),
        &recorder,
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
    let app = tauri::test::mock_app();
    let control = RuntimeControlStore::default();
    let recorder = control.status_recorder();
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
        &recorder,
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
    assert_eq!(control["runtimeStatus"]["status"], "running");
    assert_eq!(control["revision"], 1);
    Ok(())
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
fn recognition_category_keeps_the_stable_stt_wire_name() {
    let category = serde_json::to_value(DiagnosticCategory::Recognition).unwrap_or_else(
        |serialization_error| {
            serde_json::json!({ "serializationError": serialization_error.to_string() })
        },
    );

    assert_eq!(category, "stt");
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
        AppError::recognition("x"),
        AppError::recognition_provider(ProviderFailureClass::Unknown, "x"),
        AppError::recognition_backpressure("x"),
        AppError::recognition_network_terminal("x"),
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
