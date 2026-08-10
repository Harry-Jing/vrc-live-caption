use super::super::test_support::{
    RecordingChatboxTransport, receive_json_event, runtime_test_publisher,
};
use super::*;
use crate::config::OpenAiTranscriptionModel;
use crate::recognition::{ScriptedRecognitionAdapter, ScriptedRecognitionContext, ScriptedText};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tauri::Listener;

fn scripted_context(
    generation: &RuntimeGeneration,
    model: OpenAiTranscriptionModel,
) -> ScriptedRecognitionContext {
    ScriptedRecognitionContext {
        generation: generation.generation_id(),
        stream_id: generation.stream_id().to_string(),
        language: Some("en".to_string()),
        model: model.as_str().to_string(),
    }
}

#[test]
fn recognition_events_fan_out_to_the_aggregate_and_completed_publisher() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let caption_session = CaptionSessionStore::default();
    let generation = RuntimeGeneration::activate(app.handle(), 1, caption_session.clone())?;
    let (publisher, text_receiver) = runtime_test_publisher(generation.clone(), None)?;
    let events = ScriptedRecognitionAdapter::new(scripted_context(
        &generation,
        OpenAiTranscriptionModel::GptLiveTranscribe,
    ))
    .script_unit(
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

    let snapshot = caption_session.snapshot()?;
    assert!(snapshot.active_units.is_empty());
    assert_eq!(snapshot.captions.len(), 1);
    assert_eq!(snapshot.captions[0].revision, 3);
    assert_eq!(snapshot.captions[0].text, "full completed text");
    assert_eq!(
        snapshot.captions[0].state,
        crate::caption_session::CaptionState::Completed
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
    let caption_session = CaptionSessionStore::default();
    let generation = RuntimeGeneration::activate(app.handle(), 1, caption_session.clone())?;
    let (publisher, text_receiver) = runtime_test_live_publisher(generation.clone())?;
    let events = ScriptedRecognitionAdapter::new(scripted_context(
        &generation,
        OpenAiTranscriptionModel::GptLiveTranscribe,
    ))
    .script_unit(
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
    assert_eq!(caption_session.snapshot()?.captions[0].revision, 1);

    publisher.request_close(PublisherCloseReason::RuntimeError)?;
    publisher.join()?;
    Ok(())
}

#[test]
fn fan_out_rejects_out_of_order_duplicate_stopped_and_old_generation_events() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let caption_session = CaptionSessionStore::default();
    let first = RuntimeGeneration::activate(app.handle(), 1, caption_session.clone())?;
    let events = ScriptedRecognitionAdapter::new(scripted_context(
        &first,
        OpenAiTranscriptionModel::GptLiveTranscribe,
    ))
    .script_unit(
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

    let second = RuntimeGeneration::activate(app.handle(), 2, caption_session.clone())?;
    assert_eq!(
        second.submit_recognition_event(app.handle(), None, events[3].clone())?,
        RecognitionEventSubmitOutcome::Ignored
    );
    assert!(
        caption_session
            .snapshot()?
            .captions
            .iter()
            .all(|caption| caption.generation != 2)
    );
    second.request_stop(None)?;
    Ok(())
}

#[test]
fn reconnect_boundary_aborts_active_units_without_closing_the_generation() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let caption_session = CaptionSessionStore::default();
    let generation = RuntimeGeneration::activate(app.handle(), 1, caption_session.clone())?;

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

    generation.abort_active_units_for_reconnect(app.handle(), None)?;

    assert!(caption_session.snapshot()?.active_units.is_empty());
    assert!(generation.commit_if_active(|| {})?);
    assert!(!generation.is_work_cancelled());
    Ok(())
}

#[test]
fn unit_ended_events_close_the_app_unit_and_completed_typing_activity() -> AppResult<()> {
    for (index, (reason, expected_reason)) in [
        (RecognitionEndReason::NoSpeech, "noSpeech"),
        (
            RecognitionEndReason::Failed {
                detail: "provider item failed (code=test_failure)".to_string(),
            },
            "sttFailed",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let app = tauri::test::mock_app();
        let (ended_sender, ended_receiver) = std::sync::mpsc::channel();
        app.listen("utterance-ended", move |event| {
            let _ = ended_sender.send(event.payload().to_string());
        });
        let (diagnostic_sender, diagnostic_receiver) = std::sync::mpsc::channel();
        app.listen("diagnostic-event", move |event| {
            let _ = diagnostic_sender.send(event.payload().to_string());
        });
        let caption_session = CaptionSessionStore::default();
        let generation = RuntimeGeneration::activate(app.handle(), 1, caption_session.clone())?;
        let (typing_sender, typing_receiver) = std::sync::mpsc::channel();
        let (publisher, text_receiver) =
            runtime_test_publisher(generation.clone(), Some(typing_sender))?;
        let unit_id = format!("terminal-unit-{index}");
        let events = ScriptedRecognitionAdapter::new(scripted_context(
            &generation,
            OpenAiTranscriptionModel::GptLiveTranscribe,
        ))
        .script_ended(unit_id.clone(), 400, reason);

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

        let ended = receive_json_event(&ended_receiver, "utterance-ended")?;
        assert_eq!(ended["utteranceId"], unit_id);
        assert_eq!(ended["reason"], expected_reason);
        if expected_reason == "sttFailed" {
            let diagnostic = receive_json_event(&diagnostic_receiver, "diagnostic-event")?;
            assert_eq!(diagnostic["code"], "stt.item_failed");
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
        let snapshot = caption_session.snapshot()?;
        assert!(snapshot.active_units.is_empty());
        assert!(snapshot.captions.is_empty());
        assert!(text_receiver.try_recv().is_err());

        publisher.request_close(PublisherCloseReason::RuntimeError)?;
        publisher.join()?;
    }
    Ok(())
}

#[test]
fn stop_cancels_work_before_waiting_for_an_app_commit() -> AppResult<()> {
    let generation = RuntimeGeneration::active();
    let commit_generation = generation.clone();
    let stop_generation = generation.clone();
    let (commit_started_sender, commit_started_receiver) = std::sync::mpsc::channel();
    let (release_commit_sender, release_commit_receiver) = std::sync::mpsc::channel();

    let commit = thread::spawn(move || {
        commit_generation.commit_if_active(|| {
            let _ = commit_started_sender.send(());
            let _ = release_commit_receiver.recv();
        })
    });
    commit_started_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("App commit did not reach the test boundary."))?;

    let stop = thread::spawn(move || stop_generation.request_stop(None));
    let deadline = Instant::now() + Duration::from_secs(1);
    let cancelled_before_commit_finished = loop {
        if generation.is_work_cancelled() {
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
    );
    stop.join()
        .map_err(|_| AppError::runtime("Runtime stop test thread panicked."))??;
    assert!(cancelled_before_commit_finished);
    assert!(!generation.commit_if_active(|| {})?);
    Ok(())
}

#[test]
fn poisoned_generation_gate_still_closes_and_joins_the_publisher() -> AppResult<()> {
    let generation = RuntimeGeneration::active();
    let output_gate = generation.test_output_gate();
    let poisoner = thread::spawn(move || {
        if let Ok(_gate) = output_gate.lock() {
            std::panic::resume_unwind(Box::new("poison generation gate for shutdown coverage"));
        }
    });
    assert!(poisoner.join().is_err());
    let (publisher, text_receiver) = runtime_test_publisher(generation.clone(), None)?;

    assert!(generation.request_stop(Some(&publisher)).is_err());
    publisher.join()?;
    assert_eq!(
        publisher.try_submit_completed_event(CompletedPublisherEvent::Completed {
            unit_id: "late".to_string(),
            text: "late".to_string(),
        })?,
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
    let caption_session = CaptionSessionStore::default();
    let first = RuntimeGeneration::activate(app.handle(), 1, caption_session.clone())?;
    let (first_publisher, first_text_receiver) = runtime_test_publisher(first.clone(), None)?;
    let first_events = ScriptedRecognitionAdapter::new(scripted_context(
        &first,
        OpenAiTranscriptionModel::GptTranscribe,
    ))
    .script_unit("old", 100, &[], ScriptedText::new("late old caption", 200));
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

    let second = RuntimeGeneration::activate(app.handle(), 2, caption_session.clone())?;
    let (second_publisher, second_text_receiver) = runtime_test_publisher(second.clone(), None)?;
    let second_events = ScriptedRecognitionAdapter::new(scripted_context(
        &second,
        OpenAiTranscriptionModel::GptTranscribe,
    ))
    .script_unit(
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
    let snapshot = caption_session.snapshot()?;
    assert_eq!(snapshot.captions.len(), 1);
    assert_eq!(snapshot.captions[0].generation, 2);
    assert_eq!(snapshot.captions[0].text, "current caption");

    second_publisher.request_close(PublisherCloseReason::RuntimeError)?;
    second_publisher.join()?;
    Ok(())
}

fn runtime_test_live_publisher(
    generation: RuntimeGeneration,
) -> AppResult<(RuntimeChatboxPublisher, std::sync::mpsc::Receiver<String>)> {
    let (text_sender, text_receiver) = std::sync::mpsc::channel();
    let reporter: LivePublisherReporter = Arc::new(|_| {});
    let publisher = LiveChatboxPublisher::start(
        Arc::new(RecordingChatboxTransport {
            text_sender,
            typing_sender: None,
        }),
        ChatboxPacer::default(),
        generation,
        ResolvedPublicationPolicy::LiveUnit {
            observation_window_ms: 1_000,
        },
        reporter,
    )?;

    Ok((RuntimeChatboxPublisher::Live(publisher), text_receiver))
}
