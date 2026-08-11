use super::super::test_support::{
    RecordingChatboxTransport, inactive_caption_update, receive_json_event, runtime_test_publisher,
};
use super::*;
use crate::recognition::{ScriptedRecognitionContext, ScriptedRecognitionEvents, ScriptedText};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tauri::Listener;

fn scripted_context(generation: &RuntimeGeneration) -> ScriptedRecognitionContext {
    ScriptedRecognitionContext {
        generation: generation.generation_id(),
        stream_id: generation.stream_id().to_string(),
        language: Some("en".to_string()),
    }
}

#[test]
fn recognition_events_fan_out_to_the_aggregate_and_completed_publisher() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let caption_aggregate = CaptionAggregateStore::default();
    let generation = RuntimeGeneration::activate(app.handle(), 1, caption_aggregate.clone())?;
    let (publisher, text_receiver) = runtime_test_publisher(generation.clone(), None)?;
    let events = ScriptedRecognitionEvents::new(scripted_context(&generation)).script_unit(
        "scripted-unit",
        100,
        &[
            ScriptedText::new("full", 120),
            ScriptedText::new("full ongoing text", 140),
        ],
        ScriptedText::new("full completed text", 160),
    );

    for event in events {
        assert_eq!(
            generation.submit_recognition_event(app.handle(), Some(&publisher), event)?,
            RecognitionEventSubmitOutcome::Accepted
        );
    }

    let snapshot = caption_aggregate.snapshot()?;
    assert!(snapshot.open_source_units.is_empty());
    assert_eq!(snapshot.captions.len(), 1);
    assert_eq!(snapshot.captions[0].revision, 3);
    assert_eq!(snapshot.captions[0].text, "full completed text");
    assert_eq!(
        snapshot.captions[0].state,
        crate::caption::CaptionState::Completed
    );
    assert_eq!(
        text_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Completed caption was not published."))?,
        "full completed text"
    );
    assert!(text_receiver.try_recv().is_err());

    publisher.request_close(PublisherCloseReason::RuntimeError)?;
    publisher.join()?;
    Ok(())
}

#[test]
fn accepted_recognition_aggregate_fans_out_to_the_live_publisher() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let caption_aggregate = CaptionAggregateStore::default();
    let generation = RuntimeGeneration::activate(app.handle(), 1, caption_aggregate.clone())?;
    let (publisher, text_receiver) = runtime_test_live_publisher(generation.clone())?;
    let events = ScriptedRecognitionEvents::new(scripted_context(&generation)).script_unit(
        "short-live-unit",
        100,
        &[],
        ScriptedText::new("short completed live text", 150),
    );

    for event in events {
        assert_eq!(
            generation.submit_recognition_event(app.handle(), Some(&publisher), event)?,
            RecognitionEventSubmitOutcome::Accepted
        );
    }

    assert_eq!(
        text_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Accepted aggregate did not reach Live output."))?,
        "short completed live text"
    );
    assert_eq!(caption_aggregate.snapshot()?.captions[0].revision, 1);

    publisher.request_close(PublisherCloseReason::RuntimeError)?;
    publisher.join()?;
    Ok(())
}

#[test]
fn fan_out_rejects_out_of_order_duplicate_stopped_and_old_generation_events() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let caption_aggregate = CaptionAggregateStore::default();
    let first = RuntimeGeneration::activate(app.handle(), 1, caption_aggregate.clone())?;
    let events = ScriptedRecognitionEvents::new(scripted_context(&first)).script_unit(
        "ordered-unit",
        200,
        &[
            ScriptedText::new("revision one", 210),
            ScriptedText::new("revision two full", 220),
        ],
        ScriptedText::new("revision three completed", 230),
    );

    assert_eq!(
        first.submit_recognition_event(app.handle(), None, events[0].clone())?,
        RecognitionEventSubmitOutcome::Accepted
    );
    assert_eq!(
        first.submit_recognition_event(app.handle(), None, events[2].clone())?,
        RecognitionEventSubmitOutcome::Accepted
    );
    assert_eq!(
        first.submit_recognition_event(app.handle(), None, events[1].clone())?,
        RecognitionEventSubmitOutcome::Ignored
    );
    assert_eq!(
        first.submit_recognition_event(app.handle(), None, events[3].clone())?,
        RecognitionEventSubmitOutcome::Accepted
    );
    assert_eq!(
        first.submit_recognition_event(app.handle(), None, events[3].clone())?,
        RecognitionEventSubmitOutcome::Ignored
    );
    first.request_stop(None)?;
    assert_eq!(
        first.submit_recognition_event(app.handle(), None, events[2].clone())?,
        RecognitionEventSubmitOutcome::Stopped
    );

    let second = RuntimeGeneration::activate(app.handle(), 2, caption_aggregate.clone())?;
    assert_eq!(
        second.submit_recognition_event(app.handle(), None, events[3].clone())?,
        RecognitionEventSubmitOutcome::Ignored
    );
    assert!(
        caption_aggregate
            .snapshot()?
            .captions
            .iter()
            .all(|caption| caption.generation != 2)
    );
    second.request_stop(None)?;
    Ok(())
}

#[test]
fn reconnect_boundary_aborts_open_source_units_without_closing_the_generation() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let caption_aggregate = CaptionAggregateStore::default();
    let generation = RuntimeGeneration::activate(app.handle(), 1, caption_aggregate.clone())?;

    for unit_id in ["pending-one", "pending-two"] {
        assert_eq!(
            generation.submit_recognition_event(
                app.handle(),
                None,
                RecognitionEvent::UnitStarted {
                    generation: generation.generation_id(),
                    stream_id: generation.stream_id().to_string(),
                    unit_id: unit_id.to_string(),
                    started_at_ms: 100,
                },
            )?,
            RecognitionEventSubmitOutcome::Accepted
        );
    }

    generation.abort_open_source_units_for_reconnect(app.handle(), None)?;

    assert!(caption_aggregate.snapshot()?.open_source_units.is_empty());
    assert!(generation.commit_if_active(|| {})?);
    assert!(!generation.is_work_cancelled());
    Ok(())
}

#[test]
fn unit_aborted_events_close_the_app_unit_and_completed_typing_activity() -> AppResult<()> {
    for (index, (reason, expects_diagnostic)) in [
        (RecognitionUnitAbortReason::NoSpeech, false),
        (
            RecognitionUnitAbortReason::Failed {
                detail: "provider item failed (code=test_failure)".to_string(),
            },
            true,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let app = tauri::test::mock_app();
        let (diagnostic_sender, diagnostic_receiver) = std::sync::mpsc::channel();
        app.listen("diagnostic-event", move |event| {
            let _ = diagnostic_sender.send(event.payload().to_string());
        });
        let caption_aggregate = CaptionAggregateStore::default();
        let generation = RuntimeGeneration::activate(app.handle(), 1, caption_aggregate.clone())?;
        let (typing_sender, typing_receiver) = std::sync::mpsc::channel();
        let (publisher, text_receiver) =
            runtime_test_publisher(generation.clone(), Some(typing_sender))?;
        let unit_id = format!("terminal-unit-{index}");
        let events = ScriptedRecognitionEvents::new(scripted_context(&generation)).script_aborted(
            unit_id.clone(),
            400,
            reason,
        );

        assert_eq!(
            generation.submit_recognition_event(
                app.handle(),
                Some(&publisher),
                events[0].clone(),
            )?,
            RecognitionEventSubmitOutcome::Accepted
        );
        assert!(
            typing_receiver
                .recv_timeout(Duration::from_secs(1))
                .map_err(|_| AppError::runtime("Typing indicator did not turn on."))?
        );
        assert_eq!(
            generation.submit_recognition_event(
                app.handle(),
                Some(&publisher),
                events[1].clone(),
            )?,
            RecognitionEventSubmitOutcome::Accepted
        );

        if expects_diagnostic {
            let diagnostic = receive_json_event(&diagnostic_receiver, "diagnostic-event")?;
            assert_eq!(diagnostic["code"], "stt.item_failed");
            assert_eq!(
                diagnostic["message"],
                "One caption unit could not be transcribed"
            );
            assert!(
                diagnostic["detail"]
                    .as_str()
                    .is_some_and(|detail| detail.contains("code=test_failure"))
            );
        } else {
            assert!(diagnostic_receiver.try_recv().is_err());
        }
        assert!(
            !typing_receiver
                .recv_timeout(Duration::from_secs(1))
                .map_err(|_| AppError::runtime("Typing indicator did not turn off."))?
        );
        let snapshot = caption_aggregate.snapshot()?;
        assert!(snapshot.open_source_units.is_empty());
        assert!(snapshot.captions.is_empty());
        assert!(text_receiver.try_recv().is_err());

        publisher.request_close(PublisherCloseReason::RuntimeError)?;
        publisher.join()?;
    }
    Ok(())
}

#[test]
fn hard_stop_cutoff_rejects_caption_commits_before_output_shutdown_finishes() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let caption_aggregate = CaptionAggregateStore::default();
    let generation = RuntimeGeneration::activate(app.handle(), 1, caption_aggregate.clone())?;

    // This is the first action in request_stop. Output shutdown has not yet
    // started, but App and Chatbox commits must already share the Stop cutoff.
    generation.generation_fence.request_stop();
    assert!(!generation.accepts_new_work());
    assert!(generation.committer().is_closed());

    let outcome = generation.submit_recognition_event(
        app.handle(),
        None,
        RecognitionEvent::UnitStarted {
            generation: generation.generation_id(),
            stream_id: generation.stream_id().to_string(),
            unit_id: "after-stop-intent".to_string(),
            started_at_ms: 1,
        },
    )?;

    assert_eq!(outcome, RecognitionEventSubmitOutcome::Stopped);
    assert!(caption_aggregate.snapshot()?.open_source_units.is_empty());
    Ok(())
}

#[test]
fn generation_committer_finishes_a_linearized_commit_before_stop_and_closes_future_commits()
-> AppResult<()> {
    let generation = RuntimeGeneration::active();
    let committer = generation.committer();
    let commit_committer = committer.clone();
    let stop_generation = generation.clone();
    let (commit_started_sender, commit_started_receiver) = std::sync::mpsc::channel();
    let (release_commit_sender, release_commit_receiver) = std::sync::mpsc::channel();

    let commit = thread::spawn(move || {
        commit_committer.try_commit(|| {
            let _ = commit_started_sender.send(());
            let _ = release_commit_receiver.recv();
        })
    });
    commit_started_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("App commit did not reach the test boundary."))?;

    let stop = thread::spawn(move || stop_generation.request_stop(None));
    let deadline = Instant::now() + Duration::from_secs(1);
    let closed_before_commit_finished = loop {
        if committer.is_closed() {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        thread::sleep(Duration::from_millis(1));
    };

    release_commit_sender
        .send(())
        .map_err(|_| AppError::runtime("Could not release the App commit."))?;
    assert!(
        commit
            .join()
            .map_err(|_| AppError::runtime("App commit test thread panicked."))??
            .is_some()
    );
    stop.join()
        .map_err(|_| AppError::runtime("Runtime stop test thread panicked."))??;
    assert!(closed_before_commit_finished);
    assert!(generation.is_work_cancelled());
    assert!(committer.try_commit(|| {})?.is_none());
    Ok(())
}

#[test]
fn poisoned_generation_gate_still_closes_and_joins_the_publisher() -> AppResult<()> {
    let generation = RuntimeGeneration::active();
    let poison_generation = generation.clone();
    let poisoner = thread::spawn(move || {
        poison_generation.poison_commit_gate_for_test();
    });
    assert!(poisoner.join().is_err());
    let (publisher, text_receiver) = runtime_test_publisher(generation.clone(), None)?;

    assert!(generation.request_stop(Some(&publisher)).is_err());
    publisher.join()?;
    assert_eq!(
        publisher.try_submit(&inactive_caption_update(1))?,
        PublisherSubmitOutcome::Closed
    );
    assert!(matches!(
        text_receiver.recv_timeout(Duration::from_millis(50)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    Ok(())
}

#[test]
fn stopped_generation_cannot_publish_while_a_new_generation_can() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let caption_aggregate = CaptionAggregateStore::default();
    let first = RuntimeGeneration::activate(app.handle(), 1, caption_aggregate.clone())?;
    let (first_publisher, first_text_receiver) = runtime_test_publisher(first.clone(), None)?;
    let first_events = ScriptedRecognitionEvents::new(scripted_context(&first)).script_unit(
        "old",
        100,
        &[],
        ScriptedText::new("late old caption", 200),
    );
    assert_eq!(
        first.submit_recognition_event(
            app.handle(),
            Some(&first_publisher),
            first_events[0].clone(),
        )?,
        RecognitionEventSubmitOutcome::Accepted
    );
    first.request_stop(Some(&first_publisher))?;
    first_publisher.join()?;
    assert_eq!(
        first.submit_recognition_event(
            app.handle(),
            Some(&first_publisher),
            first_events[1].clone(),
        )?,
        RecognitionEventSubmitOutcome::Stopped
    );

    let second = RuntimeGeneration::activate(app.handle(), 2, caption_aggregate.clone())?;
    let (second_publisher, second_text_receiver) = runtime_test_publisher(second.clone(), None)?;
    let second_events = ScriptedRecognitionEvents::new(scripted_context(&second)).script_unit(
        "current",
        300,
        &[],
        ScriptedText::new("current caption", 400),
    );
    for event in second_events {
        assert_eq!(
            second.submit_recognition_event(app.handle(), Some(&second_publisher), event)?,
            RecognitionEventSubmitOutcome::Accepted
        );
    }

    assert!(matches!(
        first_text_receiver.recv_timeout(Duration::from_millis(50)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    assert_eq!(
        second_text_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Current generation did not publish."))?,
        "current caption"
    );
    let snapshot = caption_aggregate.snapshot()?;
    assert_eq!(snapshot.captions.len(), 1);
    assert_eq!(snapshot.captions[0].generation, 2);
    assert_eq!(snapshot.captions[0].text, "current caption");

    second_publisher.request_close(PublisherCloseReason::RuntimeError)?;
    second_publisher.join()?;
    Ok(())
}

fn runtime_test_live_publisher(
    generation: RuntimeGeneration,
) -> AppResult<(ChatboxPublication, std::sync::mpsc::Receiver<String>)> {
    let (text_sender, text_receiver) = std::sync::mpsc::channel();
    let reporter: Arc<dyn Fn(DiagnosticUpdate) + Send + Sync> = Arc::new(|_| {});
    let publication = ChatboxPublication::start_with_transport(
        Arc::new(RecordingChatboxTransport {
            text_sender,
            typing_sender: None,
        }),
        ChatboxPacer::default(),
        generation.generation_id(),
        generation.committer(),
        ResolvedPublicationTiming::LiveUnit {
            observation_window_ms: 1_000,
        },
        reporter,
    )?;

    Ok((publication, text_receiver))
}
