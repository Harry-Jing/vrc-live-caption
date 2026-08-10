use super::*;
use crate::error::{AppError, AppResult};
use crate::events::emit_diagnostic;
use std::time::Duration;
use tauri::Listener;

fn receive_json_event(
    receiver: &std::sync::mpsc::Receiver<String>,
    event_name: &str,
) -> AppResult<serde_json::Value> {
    let payload = receiver.recv_timeout(Duration::from_secs(1)).map_err(|_| {
        AppError::runtime(format!("Did not receive the expected {event_name} event."))
    })?;

    serde_json::from_str(&payload).map_err(|error| {
        AppError::runtime(format!("Failed to parse the {event_name} event: {error}"))
    })
}

#[test]
fn publisher_diagnostics_keep_stable_osc_wire_codes() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let (diagnostic_sender, diagnostic_receiver) = std::sync::mpsc::channel();
    app.listen("diagnostic-event", move |event| {
        let _ = diagnostic_sender.send(event.payload().to_string());
    });
    let diagnostics = vec![
        (
            PublisherDiagnostic::UnitPublished {
                unit_id: "published".to_string(),
                page_count: 2,
                byte_count: 42,
                target: "127.0.0.1:9000".to_string(),
            },
            "osc.completed_unit_sent",
            "info",
        ),
        (
            PublisherDiagnostic::UnitDroppedOverload {
                unit_id: "dropped".to_string(),
                page_count: 2,
            },
            "osc.completed_unit_dropped_overload",
            "warning",
        ),
        (
            PublisherDiagnostic::UnitRejectedOverload {
                unit_id: "rejected".to_string(),
                page_count: 33,
            },
            "osc.completed_unit_rejected_overload",
            "warning",
        ),
        (
            PublisherDiagnostic::UnitExpired {
                unit_id: "expired".to_string(),
                page_count: 2,
            },
            "osc.completed_unit_expired",
            "warning",
        ),
        (
            PublisherDiagnostic::LayoutFailed {
                unit_id: "layout".to_string(),
                reason: "test layout failure".to_string(),
            },
            "osc.completed_layout_failed",
            "warning",
        ),
        (
            PublisherDiagnostic::UnitSendFailed {
                unit_id: "send".to_string(),
                page_index: 2,
                page_count: 3,
                pages_sent: 1,
                error: AppError::osc_send("test", "send failure".to_string()),
            },
            "osc.send_failed",
            "error",
        ),
        (
            PublisherDiagnostic::PagesDiscardedOnClose {
                reason: PublisherCloseReason::Stop,
                unit_count: 2,
                page_count: 3,
                started_unit_count: 1,
            },
            "osc.completed_pages_discarded_on_stop",
            "info",
        ),
        (
            PublisherDiagnostic::PagesDiscardedOnClose {
                reason: PublisherCloseReason::RuntimeError,
                unit_count: 2,
                page_count: 3,
                started_unit_count: 1,
            },
            "osc.completed_pages_discarded_on_error",
            "info",
        ),
        (
            PublisherDiagnostic::TypingFailed {
                is_typing: false,
                error: AppError::osc_send("test", "typing failure".to_string()),
            },
            "osc.send_failed",
            "error",
        ),
        (
            PublisherDiagnostic::WorkerFailed {
                reason: "worker failure".to_string(),
            },
            "osc.completed_publisher_failed",
            "error",
        ),
    ];

    for (diagnostic, expected_code, expected_severity) in diagnostics {
        emit_diagnostic(app.handle(), completed_publisher_diagnostic(diagnostic));
        let event = receive_json_event(&diagnostic_receiver, "Publisher diagnostic")?;
        assert_eq!(event["category"], "osc");
        assert_eq!(event["code"], expected_code);
        assert_eq!(event["severity"], expected_severity);
        if expected_code == "osc.completed_unit_rejected_overload" {
            assert!(
                event["detail"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("No partial pages were queued")
            );
        }
        if expected_code == "osc.completed_pages_discarded_on_stop" {
            assert!(
                event["detail"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("Discarded 3 unsent page(s)")
            );
        }
    }
    Ok(())
}

#[test]
fn live_publisher_diagnostics_keep_stable_osc_wire_codes() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let (diagnostic_sender, diagnostic_receiver) = std::sync::mpsc::channel();
    app.listen("diagnostic-event", move |event| {
        let _ = diagnostic_sender.send(event.payload().to_string());
    });
    let diagnostics = vec![
        (
            LivePublisherDiagnostic::ViewPublished {
                stream_id: "recognition-1-1".to_string(),
                unit_id: Some("unit-1".to_string()),
                revision: 2,
                byte_count: 12,
                target: "127.0.0.1:9000".to_string(),
            },
            "osc.live_view_sent",
            "info",
        ),
        (
            LivePublisherDiagnostic::ViewSendFailed {
                stream_id: "recognition-1-1".to_string(),
                unit_id: None,
                revision: 3,
                error: AppError::osc_send("test", "send failure".to_string()),
            },
            "osc.live_view_send_failed",
            "error",
        ),
        (
            LivePublisherDiagnostic::LayoutFailed {
                stream_id: "recognition-1-1".to_string(),
                unit_id: Some("unit-2".to_string()),
                revision: 4,
                reason: "layout failure".to_string(),
            },
            "osc.live_layout_failed",
            "warning",
        ),
        (
            LivePublisherDiagnostic::DraftDiscardedOnClose {
                reason: PublisherCloseReason::Stop,
            },
            "osc.live_draft_discarded_on_stop",
            "info",
        ),
        (
            LivePublisherDiagnostic::DraftDiscardedOnClose {
                reason: PublisherCloseReason::RuntimeError,
            },
            "osc.live_draft_discarded_on_error",
            "info",
        ),
        (
            LivePublisherDiagnostic::TypingFailed {
                error: AppError::osc_send("test", "typing failure".to_string()),
            },
            "osc.live_typing_failed",
            "error",
        ),
        (
            LivePublisherDiagnostic::WorkerFailed {
                reason: "worker failure".to_string(),
            },
            "osc.live_publisher_failed",
            "error",
        ),
    ];

    for (diagnostic, expected_code, expected_severity) in diagnostics {
        emit_diagnostic(app.handle(), live_publisher_diagnostic(diagnostic));
        let event = receive_json_event(&diagnostic_receiver, "Live publisher diagnostic")?;
        assert_eq!(event["category"], "osc");
        assert_eq!(event["code"], expected_code);
        assert_eq!(event["severity"], expected_severity);
    }
    Ok(())
}
