use super::super::test_support::{
    inactive_caption_update, receive_json_event, runtime_test_publisher,
};
use super::*;
use crate::chatbox::PublicationObservationOutcome;
use crate::error::AppError;
use crate::runtime_control::RuntimeControlStore;
use std::thread;
use std::time::Duration;
use tauri::Listener;

#[test]
fn terminal_runtime_error_records_error_and_closes_outputs_without_hard_stop() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let control = RuntimeControlStore::default();
    let status_recorder = control.status_recorder();
    let (diagnostic_sender, diagnostic_receiver) = std::sync::mpsc::channel();
    app.listen("diagnostic-event", move |event| {
        let _ = diagnostic_sender.send(event.payload().to_string());
    });
    let generation = RuntimeGeneration::active();
    let (publisher, text_receiver) = runtime_test_publisher(generation.clone(), None)?;

    supervise_runtime_thread(
        app.handle(),
        &generation,
        Some(&publisher),
        &status_recorder,
        || {
            Err(AppError::runtime(
                "Runtime test reached a terminal failure.",
            ))
        },
    );

    assert!(!generation.is_hard_stop_requested());
    assert!(!generation.commit_if_active(|| {})?);
    assert_eq!(
        publisher.try_observe(&inactive_caption_update(1))?,
        PublicationObservationOutcome::Closed
    );
    publisher.join()?;
    drop(publisher);
    assert!(matches!(
        text_receiver.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Disconnected)
    ));
    let diagnostic = receive_json_event(&diagnostic_receiver, "Runtime terminal diagnostic")?;
    assert_eq!(diagnostic["code"], "runtime.failed");
    assert_eq!(diagnostic["message"], "Runtime stopped with an error");
    assert_eq!(
        diagnostic["detail"],
        "Runtime test reached a terminal failure."
    );
    let snapshot = control.snapshot()?;
    assert_eq!(snapshot.runtime_status.status, RuntimeStatus::Error);
    assert_eq!(
        snapshot.runtime_status.message.as_deref(),
        Some("Runtime test reached a terminal failure.")
    );
    Ok(())
}

#[test]
fn runtime_thread_panic_invalidates_generation_and_closes_publisher() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let control = RuntimeControlStore::default();
    let status_recorder = control.status_recorder();
    let (diagnostic_sender, diagnostic_receiver) = std::sync::mpsc::channel();
    app.listen("diagnostic-event", move |event| {
        let _ = diagnostic_sender.send(event.payload().to_string());
    });
    let generation = RuntimeGeneration::active();
    let (publisher, text_receiver) = runtime_test_publisher(generation.clone(), None)?;
    let panic_app = app.handle().clone();
    let panic_generation = generation.clone();
    let panic_publisher = publisher.clone();

    let panicking_runtime = thread::spawn(move || {
        supervise_runtime_thread(
            &panic_app,
            &panic_generation,
            Some(&panic_publisher),
            &status_recorder,
            || -> AppResult<()> {
                std::panic::resume_unwind(Box::new("panic runtime thread for supervisor coverage"));
            },
        );
    });
    assert!(panicking_runtime.join().is_err());

    assert!(generation.is_hard_stop_requested());
    assert!(!generation.commit_if_active(|| {})?);
    publisher.join()?;
    assert_eq!(
        publisher.try_observe(&inactive_caption_update(1))?,
        PublicationObservationOutcome::Closed
    );
    assert!(matches!(
        text_receiver.recv_timeout(Duration::from_millis(50)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    let diagnostic = receive_json_event(&diagnostic_receiver, "Runtime panic diagnostic")?;
    assert_eq!(diagnostic["code"], "runtime.thread_panicked");
    assert_eq!(
        control.snapshot()?.runtime_status.status,
        RuntimeStatus::Error
    );
    Ok(())
}
