use super::*;
use crate::chatbox_publisher::{ChatboxSendReceipt, ChatboxTransport};
use crate::recognition_fakes::{
    FakeOngoingCompletedRecognitionAdapter, FakeOngoingOnlyRecognitionAdapter,
    ScriptedRecognitionContext, ScriptedText,
};
use tauri::Listener;

fn scripted_context(generation: &RuntimeGeneration, model: &str) -> ScriptedRecognitionContext {
    ScriptedRecognitionContext {
        generation: generation.generation_id(),
        stream_id: generation.stream_id().to_string(),
        language: Some("en".to_string()),
        provider: "mock".to_string(),
        model: model.to_string(),
    }
}

#[test]
fn scripted_unitful_events_fan_out_to_the_aggregate_and_completed_publisher() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let caption_session = CaptionSessionStore::default();
    let generation = RuntimeGeneration::activate(app.handle(), 1, caption_session.clone())?;
    let (publisher, text_receiver) = runtime_test_publisher(generation.clone())?;
    let events = FakeOngoingCompletedRecognitionAdapter::new(scripted_context(
        &generation,
        "fake-ongoing-completed",
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
        assert!(generation.submit_recognition_event(app.handle(), Some(&publisher), event,)?);
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
            .map_err(|_| AppError::runtime("Completed fake caption was not published."))?,
        "full completed text"
    );
    assert!(text_receiver.try_recv().is_err());

    publisher.request_close(PublisherCloseReason::RuntimeError)?;
    publisher.join()?;
    Ok(())
}

#[test]
fn scripted_fan_out_rejects_out_of_order_duplicate_stopped_and_old_generation_events()
-> AppResult<()> {
    let app = tauri::test::mock_app();
    let caption_session = CaptionSessionStore::default();
    let first = RuntimeGeneration::activate(app.handle(), 1, caption_session.clone())?;
    let events = FakeOngoingCompletedRecognitionAdapter::new(scripted_context(
        &first,
        "fake-ongoing-completed",
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

    assert!(first.submit_recognition_event(app.handle(), None, events[0].clone())?);
    assert!(first.submit_recognition_event(app.handle(), None, events[2].clone())?);
    assert!(!first.submit_recognition_event(app.handle(), None, events[1].clone())?);
    assert!(first.submit_recognition_event(app.handle(), None, events[3].clone())?);
    assert!(!first.submit_recognition_event(app.handle(), None, events[3].clone())?);
    first.request_stop(None)?;
    assert!(!first.submit_recognition_event(app.handle(), None, events[2].clone())?);

    let second = RuntimeGeneration::activate(app.handle(), 2, caption_session.clone())?;
    assert!(!second.submit_recognition_event(app.handle(), None, events[3].clone())?);
    assert!(
        caption_session
            .snapshot()?
            .captions
            .iter()
            .all(|caption| caption.generation != 2)
    );

    let unitless =
        FakeOngoingOnlyRecognitionAdapter::new(scripted_context(&second, "fake-ongoing-only"))
            .script_stream(&[
                ScriptedText::new("unitless one", 300),
                ScriptedText::new("unitless two full", 310),
            ]);
    for event in unitless {
        assert!(second.submit_recognition_event(app.handle(), None, event)?);
    }
    let active_snapshot = caption_session.snapshot()?;
    let current = active_snapshot
        .captions
        .iter()
        .find(|caption| caption.generation == 2)
        .ok_or_else(|| AppError::state("Unitless fake caption was not accepted."))?;
    assert!(current.unit_id.is_none());
    assert_eq!(current.revision, 2);
    assert_eq!(current.state, crate::caption_session::CaptionState::Ongoing);

    second.request_stop(None)?;
    assert!(
        caption_session
            .snapshot()?
            .captions
            .iter()
            .all(|caption| caption.generation != 2)
    );
    Ok(())
}

#[test]
fn runtime_mock_injection_uses_the_active_generation_fan_out() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let caption_session = CaptionSessionStore::default();
    let generation = RuntimeGeneration::activate(app.handle(), 1, caption_session.clone())?;
    let worker_generation = generation.clone();
    let join_handle = thread::spawn(move || {
        while !worker_generation.is_work_cancelled() {
            thread::sleep(Duration::from_millis(1));
        }
    });
    let manager = RuntimeManager::default();
    {
        let mut handle = manager
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
        *handle = Some(RuntimeHandle {
            generation,
            publisher: None,
            join_handle,
        });
    }

    manager.emit_mock_transcript(app.handle(), "en", "mock-model")?;

    let snapshot = caption_session.snapshot()?;
    assert!(snapshot.active_units.is_empty());
    assert_eq!(snapshot.captions.len(), 1);
    assert_eq!(
        snapshot.captions[0].text,
        "Testing live caption preview from the mock runtime."
    );
    assert_eq!(snapshot.captions[0].revision, 2);
    manager.stop(app.handle())?;
    Ok(())
}

#[test]
fn runtime_mock_injection_honors_the_unitless_ongoing_profile_across_calls() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let caption_session = CaptionSessionStore::default();
    let generation = RuntimeGeneration::activate(app.handle(), 1, caption_session.clone())?;
    let worker_generation = generation.clone();
    let join_handle = thread::spawn(move || {
        while !worker_generation.is_work_cancelled() {
            thread::sleep(Duration::from_millis(1));
        }
    });
    let manager = RuntimeManager::default();
    {
        let mut handle = manager
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
        *handle = Some(RuntimeHandle {
            generation,
            publisher: None,
            join_handle,
        });
    }

    manager.emit_mock_transcript(app.handle(), "en", MOCK_ONGOING_ONLY_MODEL)?;
    manager.emit_mock_transcript(app.handle(), "en", MOCK_ONGOING_ONLY_MODEL)?;

    let snapshot = caption_session.snapshot()?;
    assert!(snapshot.active_units.is_empty());
    assert_eq!(snapshot.captions.len(), 1);
    assert!(snapshot.captions[0].unit_id.is_none());
    assert_eq!(
        snapshot.captions[0].state,
        crate::caption_session::CaptionState::Ongoing
    );
    assert_eq!(snapshot.captions[0].revision, 4);
    assert_eq!(
        snapshot.captions[0].text,
        "Testing live caption preview from the ongoing-only mock runtime."
    );
    manager.stop(app.handle())?;
    Ok(())
}

#[test]
fn phase_one_segmenter_keeps_twenty_seconds_whole_until_silence() {
    let sample_rate = 10;
    let mut segmenter = new_phase_one_segmenter(sample_rate);
    let started_at = Instant::now();

    for sample_index in 0_u64..200 {
        let update = segmenter.push_samples(
            vec![0.2],
            started_at + Duration::from_millis(sample_index * 100),
        );
        assert!(update.ready_segment.is_none());
    }

    assert_eq!(
        segmenter.tick(started_at + Duration::from_millis(21_100)),
        Some(vec![0.2; 200])
    );
}

#[test]
fn phase_one_segmenter_continues_without_loss_after_the_thirty_second_limit() {
    let mut segmenter = new_phase_one_segmenter(10);
    let started_at = Instant::now();
    let mut speech_starts = 0;
    let mut ready_segments = Vec::new();

    for sample_index in 0_u64..400 {
        let update = segmenter.push_samples(
            vec![0.2],
            started_at + Duration::from_millis(sample_index * 100),
        );
        speech_starts += usize::from(update.speech_started);
        if let Some(samples) = update.ready_segment {
            ready_segments.push(samples);
        }
    }

    if let Some(samples) = segmenter.finish() {
        ready_segments.push(samples);
    }

    assert_eq!(speech_starts, 2);
    assert_eq!(
        ready_segments.iter().map(Vec::len).collect::<Vec<_>>(),
        vec![300, 100]
    );
    assert_eq!(ready_segments.iter().map(Vec::len).sum::<usize>(), 400);
}

#[test]
fn runtime_manager_closes_the_generation_before_joining_the_worker() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let generation = RuntimeGeneration::active();
    let manager = Arc::new(RuntimeManager::default());
    let (worker_ready_sender, worker_ready_receiver) = std::sync::mpsc::channel();
    let (release_worker_sender, release_worker_receiver) = std::sync::mpsc::channel();
    let join_handle = thread::spawn(move || {
        let _ = worker_ready_sender.send(());
        let _ = release_worker_receiver.recv();
    });

    {
        let mut handle = manager
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
        *handle = Some(RuntimeHandle {
            generation: generation.clone(),
            publisher: None,
            join_handle,
        });
    }
    worker_ready_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("Runtime test worker did not start."))?;

    let stop_manager = Arc::clone(&manager);
    let stop_app = app.handle().clone();
    let (stop_started_sender, stop_started_receiver) = std::sync::mpsc::channel();
    let stop = thread::spawn(move || {
        let _ = stop_started_sender.send(());
        stop_manager.stop(&stop_app)
    });
    stop_started_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("Runtime stop test thread did not start."))?;

    let deadline = Instant::now() + Duration::from_secs(1);
    let generation_closed_before_join = loop {
        if generation.is_hard_stopped() && !generation.commit_if_active(|| {})? {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        thread::sleep(Duration::from_millis(1));
    };

    release_worker_sender
        .send(())
        .map_err(|_| AppError::runtime("Could not release the runtime test worker."))?;
    stop.join()
        .map_err(|_| AppError::runtime("Runtime stop test thread panicked."))??;
    assert!(generation_closed_before_join);

    Ok(())
}

#[test]
fn finished_error_handle_is_reaped_before_a_restart_availability_check() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let manager = RuntimeManager::default();
    let (finished_sender, finished_receiver) = std::sync::mpsc::channel();
    let join_handle = thread::spawn(move || {
        let _ = finished_sender.send(());
    });
    finished_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("Finished runtime test thread did not exit."))?;
    let deadline = Instant::now() + Duration::from_secs(1);
    while !join_handle.is_finished() {
        if Instant::now() >= deadline {
            return Err(AppError::runtime(
                "Finished runtime test thread did not become joinable.",
            ));
        }
        thread::yield_now();
    }
    {
        let mut handle = manager
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
        *handle = Some(RuntimeHandle {
            generation: RuntimeGeneration::active(),
            publisher: None,
            join_handle,
        });
    }

    manager.ensure_start_available(app.handle())?;
    let handle = manager
        .handle
        .lock()
        .map_err(|_| AppError::state("Runtime state lock was poisoned."))?;
    assert!(handle.is_none());
    Ok(())
}

#[test]
fn stop_invalidates_an_uncommitted_start_epoch() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let manager = RuntimeManager::default();
    let expected_stop_epoch = manager.stop_epoch();

    assert!(manager.start_epoch_is_current(expected_stop_epoch));
    manager.stop(app.handle())?;
    assert!(!manager.start_epoch_is_current(expected_stop_epoch));
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
    let output_gate = Arc::clone(&generation.output_gate);
    let poisoner = thread::spawn(move || {
        if let Ok(_gate) = output_gate.lock() {
            std::panic::resume_unwind(Box::new("poison generation gate for shutdown coverage"));
        }
    });
    assert!(poisoner.join().is_err());
    let (publisher, text_receiver) = runtime_test_publisher(generation.clone())?;

    assert!(generation.request_stop(Some(&publisher)).is_err());
    publisher.join()?;
    assert_eq!(
        publisher.try_submit(CompletedPublisherEvent::Completed {
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
fn runtime_thread_panic_invalidates_generation_and_closes_publisher() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let (diagnostic_sender, diagnostic_receiver) = std::sync::mpsc::channel();
    app.listen("diagnostic-event", move |event| {
        let _ = diagnostic_sender.send(event.payload().to_string());
    });
    let generation = RuntimeGeneration::active();
    let (publisher, text_receiver) = runtime_test_publisher(generation.clone())?;
    let panic_app = app.handle().clone();
    let panic_generation = generation.clone();
    let panic_publisher = publisher.clone();

    let panicking_runtime = thread::spawn(move || {
        supervise_runtime_thread(
            &panic_app,
            &panic_generation,
            Some(&panic_publisher),
            || -> AppResult<()> {
                std::panic::resume_unwind(Box::new("panic runtime thread for supervisor coverage"));
            },
        );
    });
    assert!(panicking_runtime.join().is_err());

    assert!(generation.is_hard_stopped());
    assert!(!generation.commit_if_active(|| {})?);
    publisher.join()?;
    assert_eq!(
        publisher.try_submit(CompletedPublisherEvent::Completed {
            unit_id: "late-after-panic".to_string(),
            text: "late".to_string(),
        })?,
        PublisherSubmitOutcome::Closed
    );
    assert!(matches!(
        text_receiver.recv_timeout(Duration::from_millis(50)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    let diagnostic = receive_json_event(&diagnostic_receiver, "Runtime panic diagnostic")?;
    assert_eq!(diagnostic["code"], "runtime.thread_panicked");

    Ok(())
}

#[test]
fn stopped_generation_does_not_begin_provider_work() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let (ended_sender, ended_receiver) = std::sync::mpsc::channel();
    app.listen("utterance-ended", move |event| {
        let _ = ended_sender.send(event.payload().to_string());
    });
    let generation = RuntimeGeneration::active();
    let segment = test_speech_segment(&generation, "not-submitted")?;
    generation.request_stop(None)?;
    let provider_called = Arc::new(AtomicBool::new(false));
    let provider_called_by_worker = Arc::clone(&provider_called);
    let config = AppConfig::default();

    transcribe_and_emit_final(
        app.handle(),
        &config,
        segment,
        None,
        &generation,
        &move |_unit| {
            provider_called_by_worker.store(true, Ordering::Relaxed);
            Ok(OpenAiBoundedOutcome::NoSpeech)
        },
    )?;

    assert!(!provider_called.load(Ordering::Relaxed));
    let ended_event = receive_json_event(&ended_receiver, "discarded unsubmitted utterance")?;
    assert_eq!(ended_event["utteranceId"], "not-submitted");
    assert_eq!(ended_event["reason"], "discarded");

    Ok(())
}

#[test]
fn late_empty_and_error_results_are_discarded_after_stop() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let (ended_sender, ended_receiver) = std::sync::mpsc::channel();
    let (diagnostic_sender, diagnostic_receiver) = std::sync::mpsc::channel();
    app.listen("utterance-ended", move |event| {
        let _ = ended_sender.send(event.payload().to_string());
    });
    app.listen("diagnostic-event", move |event| {
        let _ = diagnostic_sender.send(event.payload().to_string());
    });
    let config = AppConfig::default();
    for (utterance_id, return_error) in [("late-empty", false), ("late-error", true)] {
        let generation = RuntimeGeneration::active();
        let provider_generation = generation.clone();
        let segment = test_speech_segment(&generation, utterance_id)?;
        transcribe_and_emit_final(
            app.handle(),
            &config,
            segment,
            None,
            &generation,
            &move |_unit| {
                provider_generation.request_stop(None)?;
                if return_error {
                    Err(AppError::stt("Late provider failure."))
                } else {
                    Ok(OpenAiBoundedOutcome::NoSpeech)
                }
            },
        )?;
    }

    for utterance_id in ["late-empty", "late-error"] {
        let ended_event = receive_json_event(&ended_receiver, "discarded late utterance")?;
        assert_eq!(ended_event["utteranceId"], utterance_id);
        assert_eq!(ended_event["reason"], "discarded");
    }

    let diagnostic_codes = diagnostic_receiver
        .try_iter()
        .map(|payload| {
            serde_json::from_str::<serde_json::Value>(&payload)
                .map(|event| event["code"].as_str().unwrap_or_default().to_string())
                .map_err(|error| {
                    AppError::runtime(format!("Failed to parse a diagnostic event: {error}"))
                })
        })
        .collect::<AppResult<Vec<_>>>()?;
    assert_eq!(
        diagnostic_codes
            .iter()
            .filter(|code| code.as_str() == "stt.result_discarded_on_stop")
            .count(),
        2
    );
    assert!(!diagnostic_codes.iter().any(|code| code == "stt.no_speech"));

    Ok(())
}

#[test]
fn stop_between_starting_and_mock_runtime_blocks_late_running() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let (status_sender, status_receiver) = std::sync::mpsc::channel();
    app.listen("runtime-status", move |event| {
        let _ = status_sender.send(event.payload().to_string());
    });
    let generation = RuntimeGeneration::active();

    assert!(generation.commit_if_active(|| {
        emit_status(
            app.handle(),
            RuntimeStatus::Starting,
            Some("Starting test runtime".to_string()),
        );
    })?);
    generation.request_stop(None)?;
    run_mock_runtime(app.handle().clone(), generation)?;

    let starting_event = receive_json_event(&status_receiver, "starting runtime status")?;
    assert_eq!(starting_event["status"], "starting");
    assert!(matches!(
        status_receiver.recv_timeout(Duration::from_millis(50)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));

    Ok(())
}

#[test]
fn stopped_generation_cannot_publish_while_a_new_generation_can() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let app_handle = app.handle().clone();
    let (caption_sender, caption_receiver) = std::sync::mpsc::channel();
    let (diagnostic_sender, diagnostic_receiver) = std::sync::mpsc::channel();
    let (ended_sender, ended_receiver) = std::sync::mpsc::channel();
    app.listen("caption-session-changed", move |event| {
        let _ = caption_sender.send(event.payload().to_string());
    });
    app.listen("diagnostic-event", move |event| {
        let _ = diagnostic_sender.send(event.payload().to_string());
    });
    app.listen("utterance-ended", move |event| {
        let _ = ended_sender.send(event.payload().to_string());
    });

    let caption_session = CaptionSessionStore::default();
    let generation = RuntimeGeneration::activate(app.handle(), 1, caption_session.clone())?;
    let (stopped_publisher, stopped_text_receiver) = runtime_test_publisher(generation.clone())?;
    assert_eq!(
        stopped_publisher.try_submit(CompletedPublisherEvent::Started {
            unit_id: "stopped-in-flight".to_string(),
        })?,
        PublisherSubmitOutcome::Handled
    );
    let worker_generation = generation.clone();
    let worker_publisher = stopped_publisher.clone();
    let stopped_segment =
        test_announced_speech_segment(app.handle(), &generation, "stopped-in-flight")?;
    let (in_flight_sender, in_flight_receiver) = std::sync::mpsc::channel();
    let (release_sender, release_receiver) = std::sync::mpsc::channel();
    let config = AppConfig::default();
    let worker = thread::spawn(move || {
        transcribe_and_emit_final(
            &app_handle,
            &config,
            stopped_segment,
            Some(&worker_publisher),
            &worker_generation,
            &move |unit| {
                in_flight_sender.send(()).map_err(|_| {
                    AppError::runtime("Could not announce the in-flight test segment.")
                })?;
                release_receiver.recv().map_err(|_| {
                    AppError::runtime("Could not release the in-flight test segment.")
                })?;

                Ok(completed_test_outcome(unit, "late final"))
            },
        )
    });

    in_flight_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("STT test worker did not start its segment."))?;
    generation.request_stop(Some(&stopped_publisher))?;
    stopped_publisher.join()?;

    let current_app_handle = app.handle().clone();
    let current_generation = RuntimeGeneration::activate(app.handle(), 2, caption_session.clone())?;
    let (current_publisher, current_text_receiver) =
        runtime_test_publisher(current_generation.clone())?;
    let current_config = AppConfig::default();
    let current_segment =
        test_announced_speech_segment(app.handle(), &current_generation, "current")?;

    assert_eq!(
        current_publisher.try_submit(CompletedPublisherEvent::Started {
            unit_id: "current".to_string(),
        })?,
        PublisherSubmitOutcome::Handled
    );
    transcribe_and_emit_final(
        &current_app_handle,
        &current_config,
        current_segment,
        Some(&current_publisher),
        &current_generation,
        &|unit| {
            Ok(completed_test_outcome_in_scope(
                unit,
                "current final",
                2,
                "recognition-2-1",
            ))
        },
    )?;
    let current_text = current_text_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("Current publisher did not send its caption."))?;
    assert_eq!(current_text, "current final");
    current_publisher.request_close(PublisherCloseReason::RuntimeError)?;
    current_publisher.join()?;

    release_sender
        .send(())
        .map_err(|_| AppError::runtime("Could not release the STT test worker."))?;
    worker
        .join()
        .map_err(|_| AppError::runtime("STT test worker panicked."))??;

    let completed_event = receive_completed_caption_event(&caption_receiver, "current final")?;
    assert_eq!(completed_event["active"]["generation"], 2);
    assert_eq!(completed_event["active"]["streamId"], "recognition-2-1");
    assert_eq!(completed_event["captions"][0]["unitId"], "current");
    assert_eq!(completed_event["captions"][0]["lane"], "source");
    assert_eq!(completed_event["captions"][0]["state"], "completed");
    let final_snapshot = caption_session.snapshot()?;
    assert_eq!(final_snapshot.captions.len(), 1);
    assert_eq!(final_snapshot.captions[0].text, "current final");
    let ended_event = receive_json_event(&ended_receiver, "discarded old utterance")?;
    assert_eq!(ended_event["utteranceId"], "stopped-in-flight");
    assert_eq!(ended_event["reason"], "discarded");
    assert!(matches!(
        stopped_text_receiver.recv_timeout(Duration::from_millis(50)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));

    let mut late_result_discarded = false;
    for payload in diagnostic_receiver.try_iter() {
        let event = serde_json::from_str::<serde_json::Value>(&payload).map_err(|error| {
            AppError::runtime(format!("Failed to parse a diagnostic event: {error}"))
        })?;
        late_result_discarded |= event["code"] == "stt.result_discarded_on_stop";
    }
    assert!(late_result_discarded);

    Ok(())
}

#[test]
fn runtime_error_close_preserves_an_in_flight_app_final_but_rejects_chatbox() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let app_handle = app.handle().clone();
    let (caption_sender, caption_receiver) = std::sync::mpsc::channel();
    let (diagnostic_sender, diagnostic_receiver) = std::sync::mpsc::channel();
    app.listen("caption-session-changed", move |event| {
        let _ = caption_sender.send(event.payload().to_string());
    });
    app.listen("diagnostic-event", move |event| {
        let _ = diagnostic_sender.send(event.payload().to_string());
    });

    let generation = RuntimeGeneration::activate(app.handle(), 1, CaptionSessionStore::default())?;
    let (publisher, text_receiver) = runtime_test_publisher(generation.clone())?;
    assert_eq!(
        publisher.try_submit(CompletedPublisherEvent::Started {
            unit_id: "in-flight-error".to_string(),
        })?,
        PublisherSubmitOutcome::Handled
    );
    let worker_generation = generation.clone();
    let worker_publisher = publisher.clone();
    let segment = test_announced_speech_segment(app.handle(), &generation, "in-flight-error")?;
    let (entered_sender, entered_receiver) = std::sync::mpsc::channel();
    let (release_sender, release_receiver) = std::sync::mpsc::channel();
    let config = AppConfig::default();
    let worker = thread::spawn(move || {
        transcribe_and_emit_final(
            &app_handle,
            &config,
            segment,
            Some(&worker_publisher),
            &worker_generation,
            &move |unit| {
                entered_sender.send(()).map_err(|_| {
                    AppError::runtime("Could not announce the in-flight test request.")
                })?;
                release_receiver.recv().map_err(|_| {
                    AppError::runtime("Could not release the in-flight test request.")
                })?;
                Ok(completed_test_outcome(unit, "preserved in App"))
            },
        )
    });

    entered_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("In-flight test request did not start."))?;
    generation.cancel_work();
    generation.close_publisher_at_boundary(Some(&publisher), PublisherCloseReason::RuntimeError)?;
    publisher.join()?;
    release_sender
        .send(())
        .map_err(|_| AppError::runtime("Could not release the in-flight test request."))?;
    worker
        .join()
        .map_err(|_| AppError::runtime("In-flight test worker panicked."))??;

    let completed_event = receive_completed_caption_event(&caption_receiver, "preserved in App")?;
    assert_eq!(completed_event["captions"][0]["unitId"], "in-flight-error");
    assert_eq!(completed_event["captions"][0]["lane"], "source");
    assert_eq!(completed_event["captions"][0]["revision"], 1);
    assert_eq!(completed_event["captions"][0]["state"], "completed");
    finish_runtime_output(
        app.handle(),
        &generation,
        None,
        PublisherCloseReason::RuntimeError,
    );
    let closed_snapshot = generation.caption_session.snapshot()?;
    assert!(closed_snapshot.active.is_none());
    assert_eq!(closed_snapshot.captions.len(), 1);
    assert_eq!(closed_snapshot.captions[0].text, "preserved in App");
    assert!(matches!(
        text_receiver.recv_timeout(Duration::from_millis(50)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    let diagnostic_codes = (0..2)
        .map(|_| {
            receive_json_event(&diagnostic_receiver, "in-flight Runtime error diagnostic")
                .map(|event| event["code"].as_str().unwrap_or_default().to_string())
        })
        .collect::<AppResult<Vec<_>>>()?;
    assert!(
        diagnostic_codes
            .iter()
            .any(|code| code == "osc.completed_unit_discarded_after_close")
    );

    Ok(())
}

#[test]
fn capture_error_preserves_in_flight_app_final_and_discards_queued_speech() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let app_handle = app.handle().clone();
    let (caption_sender, caption_receiver) = std::sync::mpsc::channel();
    let (ended_sender, ended_receiver) = std::sync::mpsc::channel();
    app.listen("caption-session-changed", move |event| {
        let _ = caption_sender.send(event.payload().to_string());
    });
    app.listen("utterance-ended", move |event| {
        let _ = ended_sender.send(event.payload().to_string());
    });

    let generation = RuntimeGeneration::activate(app.handle(), 1, CaptionSessionStore::default())?;
    let (segment_sender, segment_receiver) = sync_channel(STT_QUEUE_CAPACITY);
    let (in_flight_sender, in_flight_receiver) = std::sync::mpsc::channel();
    let config = AppConfig::default();

    segment_sender
        .send(test_announced_speech_segment(
            app.handle(),
            &generation,
            "in-flight",
        )?)
        .map_err(|_| AppError::runtime("Failed to queue the in-flight test segment."))?;
    segment_sender
        .send(test_announced_speech_segment(
            app.handle(),
            &generation,
            "queued",
        )?)
        .map_err(|_| AppError::runtime("Failed to queue the pending test segment."))?;

    let worker_generation = generation.clone();
    let worker = thread::spawn(move || {
        let transcribe_generation = worker_generation.clone();
        run_stt_worker(
            app_handle,
            config,
            None,
            segment_receiver,
            worker_generation,
            move |unit| {
                in_flight_sender.send(()).map_err(|_| {
                    AppError::runtime("Could not announce the in-flight test segment.")
                })?;
                while !transcribe_generation.is_work_cancelled() {
                    thread::yield_now();
                }

                Ok(completed_test_outcome(unit, "in-flight final"))
            },
        );
    });

    in_flight_receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| AppError::runtime("STT test worker did not start its first segment."))?;

    let shutdown = finish_stt_worker_after_capture(
        Err(AppError::audio("Microphone capture disconnected.")),
        generation.work_cancelled(),
        segment_sender,
        worker,
    );

    let error = shutdown
        .err()
        .ok_or_else(|| AppError::runtime("Capture failure was not returned after cleanup."))?;
    assert_eq!(error.code(), "audio.failed");

    let completed_event = receive_completed_caption_event(&caption_receiver, "in-flight final")?;
    assert_eq!(completed_event["captions"][0]["unitId"], "in-flight");
    assert_eq!(completed_event["captions"][0]["lane"], "source");
    assert_eq!(completed_event["captions"][0]["state"], "completed");

    let ended_event = receive_json_event(&ended_receiver, "discarded utterance")?;
    assert_eq!(ended_event["utteranceId"], "queued");
    assert_eq!(ended_event["reason"], "discarded");

    Ok(())
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
        emit_publisher_diagnostic(app.handle(), diagnostic);
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
fn every_no_final_resolution_emits_app_lifecycle_and_turns_typing_off() -> AppResult<()> {
    let reasons = [
        UtteranceEndReason::NoSpeech,
        UtteranceEndReason::SttFailed,
        UtteranceEndReason::Discarded,
    ];

    for (index, reason) in reasons.into_iter().enumerate() {
        let utterance_id = format!("no-final-{index}");
        let (text_sender, text_receiver) = std::sync::mpsc::channel();
        let (typing_sender, typing_receiver) = std::sync::mpsc::channel();
        let publisher = CompletedChatboxPublisher::start(
            Arc::new(RecordingChatboxTransport {
                text_sender,
                typing_sender: Some(typing_sender),
            }),
            ChatboxPacer::default(),
            RuntimeGeneration::active(),
            Arc::new(|_| {}),
        )?;
        assert_eq!(
            publisher.try_submit(CompletedPublisherEvent::Started {
                unit_id: utterance_id.clone(),
            })?,
            PublisherSubmitOutcome::Handled
        );
        assert!(
            typing_receiver
                .recv_timeout(Duration::from_secs(1))
                .map_err(|_| AppError::runtime("No-final typing indicator did not turn on."))?
        );
        let resolution = NoFinalUtteranceResolution {
            utterance_id: utterance_id.clone(),
            reason,
        };
        let mut emitted = None;

        complete_no_final_utterance(
            Some(&publisher),
            resolution,
            |emitted_utterance_id, emitted_reason| {
                emitted = Some((emitted_utterance_id, emitted_reason));
            },
        )?;

        let (emitted_utterance_id, emitted_reason) = emitted
            .ok_or_else(|| AppError::runtime("No-final completion event was not emitted."))?;
        assert_eq!(emitted_utterance_id, utterance_id);
        assert!(same_utterance_end_reason(emitted_reason, reason));
        assert!(
            !typing_receiver
                .recv_timeout(Duration::from_secs(1))
                .map_err(|_| AppError::runtime("No-final typing indicator did not turn off."))?
        );
        assert!(matches!(
            text_receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        publisher.request_close(PublisherCloseReason::RuntimeError)?;
        publisher.join()?;
    }

    Ok(())
}

fn same_utterance_end_reason(left: UtteranceEndReason, right: UtteranceEndReason) -> bool {
    matches!(
        (left, right),
        (UtteranceEndReason::NoSpeech, UtteranceEndReason::NoSpeech)
            | (UtteranceEndReason::SttFailed, UtteranceEndReason::SttFailed)
            | (UtteranceEndReason::Discarded, UtteranceEndReason::Discarded)
    )
}

struct RecordingChatboxTransport {
    text_sender: std::sync::mpsc::Sender<String>,
    typing_sender: Option<std::sync::mpsc::Sender<bool>>,
}

impl ChatboxTransport for RecordingChatboxTransport {
    fn send_text(&self, text: &str) -> AppResult<ChatboxSendReceipt> {
        self.text_sender.send(text.to_string()).map_err(|_| {
            AppError::osc_send(
                "runtime test transport",
                "Text receiver disconnected.".to_string(),
            )
        })?;

        Ok(ChatboxSendReceipt {
            target: "runtime-test".to_string(),
            byte_count: text.len(),
        })
    }

    fn send_typing(&self, is_typing: bool) -> AppResult<()> {
        if let Some(sender) = &self.typing_sender {
            sender.send(is_typing).map_err(|_| {
                AppError::osc_send(
                    "runtime test transport",
                    "Typing receiver disconnected.".to_string(),
                )
            })?;
        }
        Ok(())
    }
}

fn runtime_test_publisher(
    generation: RuntimeGeneration,
) -> AppResult<(CompletedChatboxPublisher, std::sync::mpsc::Receiver<String>)> {
    let (text_sender, text_receiver) = std::sync::mpsc::channel();
    let reporter: PublisherReporter = Arc::new(|_| {});
    let publisher = CompletedChatboxPublisher::start(
        Arc::new(RecordingChatboxTransport {
            text_sender,
            typing_sender: None,
        }),
        ChatboxPacer::default(),
        generation,
        reporter,
    )?;

    Ok((publisher, text_receiver))
}

fn test_speech_segment(
    generation: &RuntimeGeneration,
    utterance_id: &str,
) -> AppResult<CompletedAudioUnit> {
    let started_at_ms = 42;
    if generation
        .caption_session
        .start_unit(
            generation.generation_id(),
            generation.stream_id(),
            utterance_id.to_string(),
            started_at_ms,
        )?
        .is_none()
    {
        return Err(AppError::state("Test caption unit could not start."));
    }

    Ok(test_audio_unit(utterance_id, started_at_ms))
}

fn test_announced_speech_segment<R: Runtime>(
    app: &AppHandle<R>,
    generation: &RuntimeGeneration,
    utterance_id: &str,
) -> AppResult<CompletedAudioUnit> {
    let started_at_ms = 42;
    if !generation.start_caption_unit(app, utterance_id.to_string(), started_at_ms)? {
        return Err(AppError::state("Test caption unit could not be announced."));
    }

    Ok(test_audio_unit(utterance_id, started_at_ms))
}

fn test_audio_unit(utterance_id: &str, started_at_ms: u64) -> CompletedAudioUnit {
    CompletedAudioUnit {
        unit_id: utterance_id.to_string(),
        started_at_ms,
        sample_rate_hz: 16_000,
        samples: vec![0.0; 160],
    }
}

fn completed_test_outcome(unit: &CompletedAudioUnit, text: &str) -> OpenAiBoundedOutcome {
    completed_test_outcome_in_scope(unit, text, 1, "recognition-1-1")
}

fn completed_test_outcome_in_scope(
    unit: &CompletedAudioUnit,
    text: &str,
    generation: u64,
    stream_id: &str,
) -> OpenAiBoundedOutcome {
    OpenAiBoundedOutcome::Completed(CaptionSnapshotV1 {
        generation,
        stream_id: stream_id.to_string(),
        unit_id: Some(unit.unit_id.clone()),
        lane: crate::caption_session::CaptionLane::Source,
        revision: 1,
        text: text.to_string(),
        state: crate::caption_session::CaptionState::Completed,
        language: Some("en".to_string()),
        provider: "openai".to_string(),
        model: "gpt-4o-mini-transcribe".to_string(),
        unit_started_at_ms: Some(unit.started_at_ms),
        timestamp_ms: 84,
    })
}

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

fn receive_completed_caption_event(
    receiver: &std::sync::mpsc::Receiver<String>,
    expected_text: &str,
) -> AppResult<serde_json::Value> {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let payload = receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| AppError::runtime("Did not receive the completed caption snapshot."))?;
        let event = serde_json::from_str::<serde_json::Value>(&payload).map_err(|error| {
            AppError::runtime(format!(
                "Failed to parse the caption-session event: {error}"
            ))
        })?;
        let has_expected_caption = event["captions"].as_array().is_some_and(|captions| {
            captions
                .iter()
                .any(|caption| caption["state"] == "completed" && caption["text"] == expected_text)
        });
        if has_expected_caption {
            return Ok(event);
        }
    }
}
