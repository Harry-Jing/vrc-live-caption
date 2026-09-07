use super::super::test_support::{
    RecordingChatboxTransport, inactive_caption_update, receive_json_event, runtime_test_publisher,
    runtime_test_publisher_with_content,
};
use super::*;
use crate::caption::{TranslationFailureReason, TranslationUnitSnapshot};
use crate::caption_pipeline::ResolvedPublicationTiming;
use crate::config::{
    ContentSelection, TranslationConfig, TranslationEndpoint, TranslationPath, TranslationTarget,
};
use crate::credentials::{CredentialId, CredentialStorage, ResolvedCredential};
use crate::recognition::{ScriptedRecognitionContext, ScriptedRecognitionEvents, ScriptedText};
use crate::runtime::PreparedTranslation;
use crate::runtime_control::RuntimeGenerationCredentialSnapshot;
use crate::translation::{
    TestTranslationControl, TestTranslationResult, TranslationFailureClass,
    translation_module_for_test,
};
use secrecy::SecretString;
use std::sync::Arc;
use std::sync::mpsc::TryRecvError;
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

fn test_translation_selection() -> TranslationConfig {
    TranslationConfig {
        path: TranslationPath::OpenAiResponsesCompletedText,
        target: TranslationTarget::SimplifiedChinese,
        endpoint: TranslationEndpoint::Official,
    }
}

fn test_translation_credential(id: CredentialId) -> RuntimeGenerationCredentialSnapshot {
    RuntimeGenerationCredentialSnapshot {
        id,
        storage: CredentialStorage::Environment,
        display_suffix: Some("test".to_string()),
        revision: 3,
    }
}

fn test_resolved_translation_credential(id: CredentialId) -> ResolvedCredential {
    let metadata = test_translation_credential(id);
    ResolvedCredential {
        id: metadata.id,
        secret: SecretString::from("test-translation-key".to_string()),
        storage: metadata.storage,
        display_suffix: metadata.display_suffix,
    }
}

fn prepared_test_translation(
    results: impl IntoIterator<Item = TestTranslationResult>,
) -> AppResult<(PreparedTranslation, TestTranslationControl)> {
    let selection = test_translation_selection();
    let (binding, control) = translation_module_for_test(
        selection,
        test_resolved_translation_credential(CredentialId::OpenAi),
        3,
        results,
    )?;
    Ok((PreparedTranslation::cloud(binding), control))
}

fn completed_source_events(
    generation: &RuntimeGeneration,
    unit_id: &str,
    text: &str,
    started_at_ms: u64,
) -> Vec<RecognitionEvent> {
    ScriptedRecognitionEvents::new(scripted_context(generation)).script_unit(
        unit_id,
        started_at_ms,
        &[],
        ScriptedText::new(text, started_at_ms.saturating_add(1)),
    )
}

fn drain_until_terminal<R: Runtime>(
    generation: &RuntimeGeneration,
    app: &AppHandle<R>,
    chatbox_publication: Option<&ChatboxPublication>,
) -> AppResult<TranslationDrainReport> {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let report = generation.drain_translation_outcomes(app, chatbox_publication)?;
        if report.applied + report.ignored > 0 {
            return Ok(report);
        }
        if Instant::now() >= deadline {
            return Err(AppError::runtime(
                "Translation outcome did not reach Runtime in time.",
            ));
        }
        thread::yield_now();
    }
}

#[test]
fn bound_translation_rejects_a_credential_for_another_endpoint() -> AppResult<()> {
    let selection = test_translation_selection();
    let result = translation_module_for_test(
        selection,
        test_resolved_translation_credential(CredentialId::CustomTranslation),
        3,
        [],
    );

    assert!(result.is_err());
    Ok(())
}

#[test]
fn completed_source_is_published_while_translation_moves_from_pending_to_completed() -> AppResult<()>
{
    let app = tauri::test::mock_app();
    let aggregate = CaptionAggregateStore::default();
    let (prepared, control) = prepared_test_translation([TestTranslationResult::Blocked])?;
    let generation =
        RuntimeGeneration::activate_with_translation(app.handle(), 1, aggregate.clone(), prepared)?;
    let (publisher, text_receiver) = runtime_test_publisher(generation.clone(), None)?;
    let events = completed_source_events(&generation, "translated", "hello", 100);

    assert_eq!(
        generation.submit_recognition_event(app.handle(), Some(&publisher), events[0].clone())?,
        RecognitionEventSubmitOutcome::Accepted
    );
    assert_eq!(
        generation.submit_recognition_event(app.handle(), Some(&publisher), events[1].clone())?,
        RecognitionEventSubmitOutcome::Accepted
    );
    control.wait_until_called(1, Duration::from_secs(1))?;

    let pending = aggregate.snapshot()?;
    assert!(matches!(
        pending.translation_units.as_slice(),
        [TranslationUnitSnapshot::Pending { .. }]
    ));
    assert_eq!(
        text_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Source caption was not published immediately."))?,
        "hello"
    );

    control.complete_blocked(Ok("你好".to_string()));
    let report = drain_until_terminal(&generation, app.handle(), Some(&publisher))?;
    assert_eq!(report.applied, 1);
    assert_eq!(report.ignored, 0);
    assert_eq!(report.degradation, None);
    let completed = aggregate.snapshot()?;
    assert!(matches!(
        completed.translation_units.as_slice(),
        [TranslationUnitSnapshot::Completed { .. }]
    ));
    assert!(
        completed.captions.iter().any(|caption| {
            caption.lane == CaptionLane::Translation && caption.text == "你好"
        })
    );
    publisher.wait_until_text_quiescent_for_test(Duration::from_secs(1))?;
    assert!(matches!(text_receiver.try_recv(), Err(TryRecvError::Empty)));

    generation.request_stop(Some(&publisher))?;
    publisher.join()?;
    Ok(())
}

#[test]
fn ninth_translation_is_failed_as_backpressure_while_eight_are_outstanding() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let aggregate = CaptionAggregateStore::default();
    let (prepared, control) = prepared_test_translation([TestTranslationResult::Blocked])?;
    let generation =
        RuntimeGeneration::activate_with_translation(app.handle(), 1, aggregate.clone(), prepared)?;

    for index in 0_u64..8 {
        let events = completed_source_events(
            &generation,
            &format!("queued-{index}"),
            &format!("source {index}"),
            index.saturating_mul(10),
        );
        assert_eq!(
            generation.submit_recognition_event(app.handle(), None, events[0].clone())?,
            RecognitionEventSubmitOutcome::Accepted
        );
        assert_eq!(
            generation.submit_recognition_event(app.handle(), None, events[1].clone())?,
            RecognitionEventSubmitOutcome::Accepted
        );
        if index == 0 {
            control.wait_until_called(1, Duration::from_secs(1))?;
        }
    }
    let rejected = completed_source_events(&generation, "rejected", "source 8", 80);
    assert_eq!(
        generation.submit_recognition_event(app.handle(), None, rejected[0].clone())?,
        RecognitionEventSubmitOutcome::Accepted
    );
    assert_eq!(
        generation.submit_recognition_event(app.handle(), None, rejected[1].clone())?,
        RecognitionEventSubmitOutcome::AcceptedWithTranslationFailure(
            TranslationFailureReason::Backpressure,
        )
    );

    let snapshot = aggregate.snapshot()?;
    assert!(snapshot.translation_units.iter().any(|unit| matches!(
        unit,
        TranslationUnitSnapshot::Failed {
            reason_code: TranslationFailureReason::Backpressure,
            ..
        }
    )));
    assert_eq!(
        generation
            .drain_translation_outcomes(app.handle(), None)?
            .degradation,
        Some(TranslationFailureReason::Backpressure)
    );

    generation.request_stop(None)?;
    assert!(
        aggregate
            .snapshot()?
            .translation_units
            .iter()
            .all(|unit| { !matches!(unit, TranslationUnitSnapshot::Pending { .. }) })
    );
    Ok(())
}

#[test]
fn provider_failure_degrades_generation_without_blocking_a_later_translation() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let aggregate = CaptionAggregateStore::default();
    let (prepared, control) = prepared_test_translation([
        TestTranslationResult::Failed(TranslationFailureClass::ServiceUnavailable),
        TestTranslationResult::Completed("后续成功".to_string()),
    ])?;
    let generation =
        RuntimeGeneration::activate_with_translation(app.handle(), 1, aggregate.clone(), prepared)?;

    for event in completed_source_events(&generation, "failed", "first", 100) {
        assert_eq!(
            generation.submit_recognition_event(app.handle(), None, event)?,
            RecognitionEventSubmitOutcome::Accepted
        );
    }
    control.wait_until_called(1, Duration::from_secs(1))?;
    let failed = drain_until_terminal(&generation, app.handle(), None)?;
    assert_eq!(failed.applied, 1);
    assert_eq!(
        failed.degradation,
        Some(TranslationFailureReason::ProviderUnavailable)
    );

    for event in completed_source_events(&generation, "succeeded", "second", 200) {
        assert_eq!(
            generation.submit_recognition_event(app.handle(), None, event)?,
            RecognitionEventSubmitOutcome::Accepted
        );
    }
    control.wait_until_called(2, Duration::from_secs(1))?;
    let succeeded = drain_until_terminal(&generation, app.handle(), None)?;
    assert_eq!(succeeded.applied, 1);
    assert_eq!(
        succeeded.degradation,
        Some(TranslationFailureReason::ProviderUnavailable)
    );

    let snapshot = aggregate.snapshot()?;
    assert!(snapshot.translation_units.iter().any(|unit| matches!(
        unit,
        TranslationUnitSnapshot::Failed {
            reason_code: TranslationFailureReason::ProviderUnavailable,
            ..
        }
    )));
    assert!(snapshot.captions.iter().any(|caption| {
        caption.lane == CaptionLane::Translation && caption.text == "后续成功"
    }));
    generation.request_stop(None)?;
    Ok(())
}

#[test]
fn stop_records_pending_as_terminal_before_cancelling_translation_and_ignores_late_completion()
-> AppResult<()> {
    let app = tauri::test::mock_app();
    let aggregate = CaptionAggregateStore::default();
    let (prepared, control) = prepared_test_translation([TestTranslationResult::Blocked])?;
    let generation =
        RuntimeGeneration::activate_with_translation(app.handle(), 1, aggregate.clone(), prepared)?;
    for event in completed_source_events(&generation, "stopped", "private", 100) {
        assert_eq!(
            generation.submit_recognition_event(app.handle(), None, event)?,
            RecognitionEventSubmitOutcome::Accepted
        );
    }
    control.wait_until_called(1, Duration::from_secs(1))?;
    assert!(matches!(
        aggregate.snapshot()?.translation_units.as_slice(),
        [TranslationUnitSnapshot::Pending { .. }]
    ));

    generation.request_stop(None)?;
    control.complete_blocked(Ok("late".to_string()));
    let stopped = aggregate.snapshot()?;
    assert!(stopped.active_stream.is_none());
    assert!(matches!(
        stopped.translation_units.as_slice(),
        [TranslationUnitSnapshot::Failed {
            reason_code: TranslationFailureReason::Stopped,
            ..
        }]
    ));
    assert!(stopped.captions.iter().any(|caption| {
        caption.lane == CaptionLane::Source && caption.unit_id.as_deref() == Some("stopped")
    }));
    assert!(
        stopped
            .captions
            .iter()
            .all(|caption| caption.lane != CaptionLane::Translation)
    );
    assert_eq!(
        generation.drain_translation_outcomes(app.handle(), None)?,
        TranslationDrainReport::default()
    );
    Ok(())
}

#[test]
fn outcome_from_replaced_generation_is_drained_without_mutating_the_replacement() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let aggregate = CaptionAggregateStore::default();
    let (prepared, control) = prepared_test_translation([TestTranslationResult::Blocked])?;
    let first =
        RuntimeGeneration::activate_with_translation(app.handle(), 1, aggregate.clone(), prepared)?;
    for event in completed_source_events(&first, "old", "private old", 100) {
        assert_eq!(
            first.submit_recognition_event(app.handle(), None, event)?,
            RecognitionEventSubmitOutcome::Accepted
        );
    }
    control.wait_until_called(1, Duration::from_secs(1))?;

    let second = RuntimeGeneration::activate(app.handle(), 2, aggregate.clone())?;
    control.complete_blocked(Ok("late old".to_string()));
    let report = drain_until_terminal(&first, app.handle(), None)?;
    assert_eq!(report.applied, 0);
    assert_eq!(report.ignored, 1);
    let current = aggregate.snapshot()?;
    assert_eq!(
        current
            .active_stream
            .as_ref()
            .map(|stream| stream.generation),
        Some(2)
    );
    assert!(
        current
            .captions
            .iter()
            .all(|caption| caption.lane != CaptionLane::Translation)
    );
    assert!(current.translation_units.iter().any(|unit| matches!(
        unit,
        TranslationUnitSnapshot::Failed {
            reason_code: TranslationFailureReason::Stopped,
            source_ref,
        } if source_ref.generation == 1 && source_ref.unit_id == "old"
    )));

    first.request_stop(None)?;
    second.request_stop(None)?;
    Ok(())
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
        publisher.wait_until_text_quiescent_for_test(Duration::from_secs(1))?;
        assert!(matches!(text_receiver.try_recv(), Err(TryRecvError::Empty)));

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
        publisher.try_observe(&inactive_caption_update(1))?,
        PublicationObservationOutcome::Closed
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
        ChatboxTextPacer::default(),
        generation.generation_id(),
        generation.committer(),
        ResolvedPublicationTiming::LiveUnit {
            observation_window_ms: 1_000,
        },
        ContentSelection::SourceOnly,
        reporter,
    )?;

    Ok((publication, text_receiver))
}

#[test]
fn translation_only_publisher_waits_for_the_exact_terminal_translation() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let aggregate = CaptionAggregateStore::default();
    let (prepared, control) = prepared_test_translation([TestTranslationResult::Blocked])?;
    let generation =
        RuntimeGeneration::activate_with_translation(app.handle(), 1, aggregate.clone(), prepared)?;
    let (publisher, text_receiver) = runtime_test_publisher_with_content(
        generation.clone(),
        ContentSelection::TranslationOnly,
        None,
    )?;
    let events = completed_source_events(&generation, "translated", "hello", 100);

    assert_eq!(
        generation.submit_recognition_event(app.handle(), Some(&publisher), events[0].clone())?,
        RecognitionEventSubmitOutcome::Accepted
    );
    assert_eq!(
        generation.submit_recognition_event(app.handle(), Some(&publisher), events[1].clone())?,
        RecognitionEventSubmitOutcome::Accepted
    );
    control.wait_until_called(1, Duration::from_secs(1))?;

    control.complete_blocked(Ok("你好".to_string()));
    let report = drain_until_terminal(&generation, app.handle(), Some(&publisher))?;
    assert_eq!(report.applied, 1);
    assert_eq!(report.degradation, None);

    // The first and only text is the exact Translation; the held Source was
    // never published on its own.
    assert_eq!(
        text_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Translation was not published."))?,
        "你好"
    );
    publisher.wait_until_text_quiescent_for_test(Duration::from_secs(1))?;
    assert!(matches!(text_receiver.try_recv(), Err(TryRecvError::Empty)));

    generation.request_stop(Some(&publisher))?;
    publisher.join()?;
    Ok(())
}

#[test]
fn bilingual_publisher_sends_source_alone_after_a_terminal_translation_failure() -> AppResult<()> {
    let app = tauri::test::mock_app();
    let aggregate = CaptionAggregateStore::default();
    let (prepared, _control) = prepared_test_translation([TestTranslationResult::Failed(
        TranslationFailureClass::InvalidOutput,
    )])?;
    let generation =
        RuntimeGeneration::activate_with_translation(app.handle(), 1, aggregate.clone(), prepared)?;
    let (publisher, text_receiver) =
        runtime_test_publisher_with_content(generation.clone(), ContentSelection::Bilingual, None)?;
    let events = completed_source_events(&generation, "partial", "source remains", 100);

    assert_eq!(
        generation.submit_recognition_event(app.handle(), Some(&publisher), events[0].clone())?,
        RecognitionEventSubmitOutcome::Accepted
    );
    assert_eq!(
        generation.submit_recognition_event(app.handle(), Some(&publisher), events[1].clone())?,
        RecognitionEventSubmitOutcome::Accepted
    );

    let report = drain_until_terminal(&generation, app.handle(), Some(&publisher))?;
    assert_eq!(report.applied, 1);
    assert_eq!(
        report.degradation,
        Some(TranslationFailureReason::InvalidOutput)
    );
    let snapshot = aggregate.snapshot()?;
    assert!(matches!(
        snapshot.translation_units.as_slice(),
        [TranslationUnitSnapshot::Failed { .. }]
    ));

    assert_eq!(
        text_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| AppError::runtime("Source was not published as a partial result."))?,
        "source remains"
    );
    publisher.wait_until_text_quiescent_for_test(Duration::from_secs(1))?;
    assert!(matches!(text_receiver.try_recv(), Err(TryRecvError::Empty)));

    generation.request_stop(Some(&publisher))?;
    publisher.join()?;
    Ok(())
}
