use super::*;
use crate::chatbox_publisher::{ChatboxSendReceipt, ChatboxTransport};
use crate::config::{OpenAiTranscriptionModel, SttProvider};
use crate::host_resolver::{HostResolutionError, HostResolver};
use crate::recognition_fakes::{
    ScriptedRecognitionAdapter, ScriptedRecognitionContext, ScriptedText,
};
use crate::secrets::ProviderSecretStorage;
use secrecy::SecretString;
use std::io;
use std::sync::mpsc;
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
        assert!(generation.submit_recognition_event(app.handle(), Some(&publisher), event)?);
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
        assert!(generation.submit_recognition_event(app.handle(), Some(&publisher), event)?);
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
    second.request_stop(None)?;
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

        assert!(generation.submit_recognition_event(
            app.handle(),
            Some(&publisher),
            events[0].clone(),
        )?);
        assert!(
            typing_receiver
                .recv_timeout(Duration::from_secs(1))
                .map_err(|_| AppError::runtime("Typing indicator did not turn on."))?
        );
        assert!(generation.submit_recognition_event(
            app.handle(),
            Some(&publisher),
            events[1].clone(),
        )?);

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
fn stop_supersedes_a_start_blocked_in_osc_hostname_resolution() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let (diagnostic_sender, diagnostic_receiver) = mpsc::channel();
    app.listen("diagnostic-event", move |event| {
        let _ = diagnostic_sender.send(event.payload().to_string());
    });
    let manager = Arc::new(RuntimeManager::default());
    let (lookup_started_sender, lookup_started_receiver) = mpsc::sync_channel(1);
    let (lookup_release_sender, lookup_release_receiver) = mpsc::sync_channel(1);
    let lookup_release_receiver = Arc::new(Mutex::new(lookup_release_receiver));
    let worker_release = Arc::clone(&lookup_release_receiver);
    let resolver = HostResolver::with_lookup(move |_, port| {
        let _ = lookup_started_sender.send(());
        worker_release
            .lock()
            .map_err(|_| io::Error::other("Test resolver release lock was poisoned."))?
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| io::Error::other("Test resolver was not released before its timeout."))?;
        Ok(vec![std::net::SocketAddr::from(([127, 0, 0, 1], port))])
    });
    let mut config = AppConfig::default();
    config.osc.host = "blocked.test".to_string();
    let expected_stop_epoch = manager.stop_epoch();
    let request = RuntimeStartRequest {
        runtime_plan: plan_runtime(&config),
        config,
        chatbox_pacer: ChatboxPacer::default(),
        caption_session: CaptionSessionStore::default(),
        host_resolver: resolver,
        generation_id: 1,
        config_revision: 1,
        openai_api_key: SecretString::from("test-key".to_string()),
        credential: RuntimeCredentialSnapshot {
            provider: SttProvider::OpenAi,
            storage: ProviderSecretStorage::Environment,
            display_suffix: None,
            revision: 1,
        },
        expected_stop_epoch,
    };
    let start_manager = Arc::clone(&manager);
    let start_app = app.handle().clone();
    let start = thread::spawn(move || start_manager.start(start_app, request, |_| Ok(())));
    lookup_started_receiver
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| AppError::state("OSC hostname resolution did not start."))?;

    let stop_manager = Arc::clone(&manager);
    let stop_app = app.handle().clone();
    let (stop_result_sender, stop_result_receiver) = mpsc::sync_channel(1);
    let stop = thread::spawn(move || {
        let _ = stop_result_sender.send(stop_manager.stop(&stop_app));
    });
    let stop_result = stop_result_receiver
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| AppError::state("Stop waited for the blocked OS hostname lookup."))?;
    stop_result?;
    let start_outcome = start
        .join()
        .map_err(|_| AppError::state("Blocked runtime Start thread panicked."))??;

    assert_eq!(start_outcome, RuntimeStartOutcome::SupersededByStop);
    assert!(
        manager
            .handle
            .lock()
            .map_err(|_| AppError::state("Runtime state lock was poisoned."))?
            .is_none()
    );
    for payload in diagnostic_receiver.try_iter() {
        let diagnostic = serde_json::from_str::<serde_json::Value>(&payload).map_err(|error| {
            AppError::state(format!("Runtime diagnostic was not valid JSON: {error}"))
        })?;
        assert_ne!(diagnostic["code"], "osc.send_failed");
    }

    lookup_release_sender
        .send(())
        .map_err(|_| AppError::state("Could not release the blocked hostname lookup."))?;
    stop.join()
        .map_err(|_| AppError::state("Runtime Stop thread panicked."))?;
    Ok(())
}

#[test]
fn stop_cancels_an_installed_runtime_hostname_wait_before_joining() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let manager = Arc::new(RuntimeManager::default());
    let generation = RuntimeGeneration::active();
    let worker_generation = generation.clone();
    let (lookup_started_sender, lookup_started_receiver) = mpsc::sync_channel(1);
    let (lookup_release_sender, lookup_release_receiver) = mpsc::sync_channel(1);
    let lookup_release_receiver = Arc::new(Mutex::new(lookup_release_receiver));
    let worker_release = Arc::clone(&lookup_release_receiver);
    let resolver = HostResolver::with_lookup(move |_, port| {
        let _ = lookup_started_sender.send(());
        worker_release
            .lock()
            .map_err(|_| io::Error::other("Test resolver release lock was poisoned."))?
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| io::Error::other("Test resolver was not released before its timeout."))?;
        Ok(vec![std::net::SocketAddr::from(([127, 0, 0, 1], port))])
    });
    let (resolution_sender, resolution_receiver) = mpsc::sync_channel(1);
    let join_handle = thread::spawn(move || {
        let result = resolver.resolve_until(
            "blocked-openai.test",
            443,
            Instant::now() + Duration::from_secs(5),
            &|| worker_generation.is_work_cancelled(),
        );
        let _ = resolution_sender.send(result);
    });
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
    lookup_started_receiver
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| AppError::state("Installed runtime hostname lookup did not start."))?;

    let stop_manager = Arc::clone(&manager);
    let stop_app = app.handle().clone();
    let (stop_result_sender, stop_result_receiver) = mpsc::sync_channel(1);
    let stop = thread::spawn(move || {
        let _ = stop_result_sender.send(stop_manager.stop(&stop_app));
    });
    let stop_result = stop_result_receiver
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| {
            AppError::state("Stop waited for the installed runtime's OS hostname lookup.")
        })?;
    stop_result?;
    let resolution = resolution_receiver
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| {
            AppError::state("Runtime hostname wait did not observe generation cancellation.")
        })?;

    assert_eq!(resolution.err(), Some(HostResolutionError::Cancelled));
    lookup_release_sender
        .send(())
        .map_err(|_| AppError::state("Could not release the installed runtime hostname lookup."))?;
    stop.join()
        .map_err(|_| AppError::state("Installed runtime Stop thread panicked."))?;
    Ok(())
}

#[test]
fn runtime_rejects_a_plan_that_does_not_match_the_selected_model() -> AppResult<()> {
    let mut config = AppConfig::default();
    let stale_plan = plan_runtime(&config);
    config.stt.model = OpenAiTranscriptionModel::GptLiveTranscribe;

    let error = resolve_runtime_publication_policy(&config, &stale_plan)
        .err()
        .ok_or_else(|| AppError::state("Mismatched runtime plan unexpectedly started."))?;
    assert_eq!(error.code(), "config.invalid");
    assert!(error.to_string().contains("did not match"));
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
fn a_full_recognition_queue_fails_visibly_without_dropping_audio() -> AppResult<()> {
    let generation = RuntimeGeneration::active();
    let (sender, _receiver) = sync_channel(1);
    send_recognition_command(
        &generation,
        &sender,
        RecognitionCommand::Audio {
            sample_rate_hz: 24_000,
            samples: vec![0.1],
        },
    )?;

    let error = send_recognition_command(
        &generation,
        &sender,
        RecognitionCommand::Audio {
            sample_rate_hz: 24_000,
            samples: vec![0.2],
        },
    )
    .err()
    .ok_or_else(|| AppError::state("A full recognition queue silently accepted audio."))?;

    assert_eq!(error.code(), "stt.network_unreachable");
    assert!(generation.is_work_cancelled());
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
fn runtime_thread_panic_invalidates_generation_and_closes_publisher() -> AppResult<()> {
    let app = tauri::test::mock_app();
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
        publisher.try_submit_completed_event(CompletedPublisherEvent::Completed {
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
    assert!(first.submit_recognition_event(
        app.handle(),
        Some(&first_publisher),
        first_events[0].clone(),
    )?);
    first.request_stop(Some(&first_publisher))?;
    first_publisher.join()?;
    assert!(!first.submit_recognition_event(
        app.handle(),
        Some(&first_publisher),
        first_events[1].clone(),
    )?);

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
        assert!(second.submit_recognition_event(app.handle(), Some(&second_publisher), event,)?);
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
        emit_live_publisher_diagnostic(app.handle(), diagnostic);
        let event = receive_json_event(&diagnostic_receiver, "Live publisher diagnostic")?;
        assert_eq!(event["category"], "osc");
        assert_eq!(event["code"], expected_code);
        assert_eq!(event["severity"], expected_severity);
    }
    Ok(())
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
    typing_sender: Option<std::sync::mpsc::Sender<bool>>,
) -> AppResult<(RuntimeChatboxPublisher, std::sync::mpsc::Receiver<String>)> {
    let (text_sender, text_receiver) = std::sync::mpsc::channel();
    let reporter: PublisherReporter = Arc::new(|_| {});
    let publisher = CompletedChatboxPublisher::start(
        Arc::new(RecordingChatboxTransport {
            text_sender,
            typing_sender,
        }),
        ChatboxPacer::default(),
        generation,
        reporter,
    )?;

    Ok((RuntimeChatboxPublisher::Completed(publisher), text_receiver))
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
        generation.generation_id(),
        generation,
        ResolvedPublicationPolicy::LiveUnit {
            observation_window_ms: 1_000,
        },
        reporter,
    )?;

    Ok((RuntimeChatboxPublisher::Live(publisher), text_receiver))
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
