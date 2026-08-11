use super::super::test_support::{
    inactive_caption_update, receive_json_event, runtime_test_publisher,
};
use super::*;
use crate::chatbox::PublisherSubmitOutcome;
use crate::runtime_control::RuntimeControlStore;
use std::thread;
use std::time::Duration;
use tauri::Listener;

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
        publisher.try_submit(&inactive_caption_update(1))?,
        PublisherSubmitOutcome::Closed
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
