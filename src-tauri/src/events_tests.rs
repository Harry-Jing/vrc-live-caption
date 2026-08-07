use super::*;
use crate::error::AppResult;
use tauri::Listener;

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

    emit_status(
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

    emit_status(
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
        AppError::stt_network("x"),
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
