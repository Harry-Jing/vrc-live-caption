use super::*;
use crate::caption::CaptionAggregateStore;
use crate::error::{AppError, AppResult};

fn fixture_error(error: FixtureError) -> AppError {
    AppError::state(error.to_string())
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> Option<&str> {
    payload
        .downcast_ref::<&'static str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
}

fn fixture_module(
    fixture: &TranslationPolicyFixture,
) -> AppResult<(FixtureModule, TranslationOutcomeReceiver)> {
    fixture
        .start_module(TranslationTarget::SimplifiedChinese)
        .map_err(fixture_error)
}

struct PanickingScripts;

impl Iterator for PanickingScripts {
    type Item = AttemptScript;

    fn next(&mut self) -> Option<Self::Item> {
        std::panic::resume_unwind(Box::new("original script iterator failure"));
    }
}

#[test]
fn owner_proof_is_bound_to_one_fixture_and_wrong_proof_does_not_close_it() -> AppResult<()> {
    let fixture_a = TranslationPolicyFixture::new();
    let fixture_b = TranslationPolicyFixture::new();
    let (mut module_a, outcomes_a) = fixture_module(&fixture_a)?;
    let (mut module_b, _outcomes_b) = fixture_module(&fixture_b)?;
    let wrong_proof = module_b.stop_and_confirm_owner_quiesced()?;

    let error = fixture_a
        .finish(wrong_proof)
        .err()
        .ok_or_else(|| AppError::state("A foreign owner proof finished the fixture."))?;
    assert_eq!(error.kind(), FixtureErrorKind::WrongOwner);

    let store = CaptionAggregateStore::default();
    fixture_a
        .admit(
            &module_a,
            super::super::reservation(&store, 110, "after-wrong-proof", "private source")?,
            [AttemptScript::success("translated")],
        )
        .map_err(fixture_error)?;
    let _outcome = outcomes_a
        .recv_timeout(FIXTURE_WATCHDOG)
        .map_err(|_| AppError::state("Fixture closed after rejecting a foreign proof."))?;

    let proof_a = module_a.stop_and_confirm_owner_quiesced()?;
    assert_eq!(fixture_a.finish(proof_a).map_err(fixture_error)?.len(), 1);
    let proof_b = module_b.stop_and_confirm_owner_quiesced()?;
    assert!(fixture_b.finish(proof_b).map_err(fixture_error)?.is_empty());
    Ok(())
}

#[test]
fn one_fixture_rejects_a_second_owner_without_invalidating_the_first() -> AppResult<()> {
    let fixture = TranslationPolicyFixture::new();
    let (mut module, _outcomes) = fixture_module(&fixture)?;

    let error = fixture
        .start_module(TranslationTarget::SimplifiedChinese)
        .err()
        .ok_or_else(|| AppError::state("Fixture accepted a second Translation owner."))?;
    assert_eq!(error.kind(), FixtureErrorKind::OwnerAlreadyBound);

    let proof = module.stop_and_confirm_owner_quiesced()?;
    assert!(fixture.finish(proof).map_err(fixture_error)?.is_empty());
    Ok(())
}

#[test]
fn admission_rejects_another_fixtures_module_before_consuming_its_script() -> AppResult<()> {
    let fixture_a = TranslationPolicyFixture::new();
    let fixture_b = TranslationPolicyFixture::new();
    let (mut module_a, _outcomes_a) = fixture_module(&fixture_a)?;
    let (mut module_b, _outcomes_b) = fixture_module(&fixture_b)?;
    let store = CaptionAggregateStore::default();

    let error = fixture_a
        .admit(
            &module_b,
            super::super::reservation(&store, 111, "wrong-admit-owner", "private source")?,
            PanickingScripts,
        )
        .err()
        .ok_or_else(|| AppError::state("Fixture admitted work through another owner."))?;
    assert_eq!(error.kind(), FixtureErrorKind::WrongOwner);

    let proof_a = module_a.stop_and_confirm_owner_quiesced()?;
    assert!(fixture_a.finish(proof_a).map_err(fixture_error)?.is_empty());
    let proof_b = module_b.stop_and_confirm_owner_quiesced()?;
    assert!(fixture_b.finish(proof_b).map_err(fixture_error)?.is_empty());
    Ok(())
}

#[test]
fn attempt_ids_with_the_same_local_sequence_are_isolated_by_fixture() -> AppResult<()> {
    let fixture_a = TranslationPolicyFixture::new();
    let fixture_b = TranslationPolicyFixture::new();
    let (mut module_a, outcomes_a) = fixture_module(&fixture_a)?;
    let (mut module_b, outcomes_b) = fixture_module(&fixture_b)?;
    let store_a = CaptionAggregateStore::default();
    let store_b = CaptionAggregateStore::default();
    let source_a = fixture_a
        .admit(
            &module_a,
            super::super::reservation(&store_a, 116, "fixture-a-first", "first private source")?,
            [AttemptScript::held_confirmed()],
        )
        .map_err(fixture_error)?;
    let source_b = fixture_b
        .admit(
            &module_b,
            super::super::reservation(&store_b, 117, "fixture-b-first", "second private source")?,
            [AttemptScript::held_confirmed()],
        )
        .map_err(fixture_error)?;
    assert_eq!(source_a.sequence, source_b.sequence);
    assert_ne!(source_a, source_b);
    let attempt_a = fixture_a
        .wait_for_attempt_count(1, FIXTURE_WATCHDOG)
        .map_err(fixture_error)?[0]
        .id();
    let attempt_b = fixture_b
        .wait_for_attempt_count(1, FIXTURE_WATCHDOG)
        .map_err(fixture_error)?[0]
        .id();
    assert_eq!(attempt_a.sequence, attempt_b.sequence);
    assert_ne!(attempt_a, attempt_b);

    for error in [
        fixture_b.complete(attempt_a, "wrong fixture").err(),
        fixture_a
            .fail(
                attempt_b,
                TranslationFailureClass::ServiceUnavailable,
                false,
                None,
                false,
            )
            .err(),
        fixture_b.release_non_cooperative(attempt_a).err(),
        fixture_a.quiesce_unconfirmed(attempt_b).err(),
        fixture_b
            .wait_for_quiescence(attempt_a, Duration::ZERO)
            .err(),
    ] {
        let error = error.ok_or_else(|| AppError::state("Cross-fixture attempt operation ran."))?;
        assert_eq!(error.kind(), FixtureErrorKind::WrongOwner);
    }

    fixture_a
        .complete(attempt_a, "first translated")
        .map_err(fixture_error)?;
    fixture_b
        .complete(attempt_b, "second translated")
        .map_err(fixture_error)?;
    let _outcome_a = outcomes_a
        .recv_timeout(FIXTURE_WATCHDOG)
        .map_err(|_| AppError::state("Fixture A produced no outcome."))?;
    let _outcome_b = outcomes_b
        .recv_timeout(FIXTURE_WATCHDOG)
        .map_err(|_| AppError::state("Fixture B produced no outcome."))?;
    fixture_a
        .wait_for_quiescence(attempt_a, FIXTURE_WATCHDOG)
        .map_err(fixture_error)?;
    fixture_b
        .wait_for_quiescence(attempt_b, FIXTURE_WATCHDOG)
        .map_err(fixture_error)?;

    let proof_a = module_a.stop_and_confirm_owner_quiesced()?;
    let records_a = fixture_a.finish(proof_a).map_err(fixture_error)?;
    let proof_b = module_b.stop_and_confirm_owner_quiesced()?;
    let records_b = fixture_b.finish(proof_b).map_err(fixture_error)?;
    assert_eq!(records_a[0].terminal(), Some(AttemptTerminal::Completed));
    assert_eq!(records_b[0].terminal(), Some(AttemptTerminal::Completed));
    Ok(())
}

#[test]
fn delay_waiter_surfaces_a_missing_retry_script_without_masking_it_as_a_watchdog() -> AppResult<()>
{
    let fixture = TranslationPolicyFixture::new();
    let (mut module, _outcomes) = fixture_module(&fixture)?;
    let store = CaptionAggregateStore::default();
    fixture
        .admit(
            &module,
            super::super::reservation(&store, 118, "missing-retry-script", "private source")?,
            [AttemptScript::failure(
                TranslationFailureClass::ServiceUnavailable,
                true,
                None,
                false,
            )],
        )
        .map_err(fixture_error)?;
    fixture
        .wait_for_delay_count(1, FIXTURE_WATCHDOG)
        .map_err(fixture_error)?;

    std::thread::scope(|scope| -> AppResult<()> {
        let fixture_ref = &fixture;
        let waiter = scope.spawn(move || fixture_ref.wait_for_delay_count(2, FIXTURE_WATCHDOG));
        fixture
            .wait_until_delay_waiting_for_count(2, FIXTURE_WATCHDOG)
            .map_err(fixture_error)?;
        fixture
            .advance(Duration::from_millis(250))
            .map_err(fixture_error)?;

        let error = waiter
            .join()
            .map_err(|_| AppError::state("Delay waiter thread panicked."))?
            .err()
            .ok_or_else(|| {
                AppError::state("Missing retry script did not fail the delay waiter.")
            })?;
        assert_eq!(error.kind(), FixtureErrorKind::MissingScript);
        Ok(())
    })?;

    let proof = module.stop_and_confirm_owner_quiesced()?;
    let sticky = fixture
        .finish(proof)
        .err()
        .ok_or_else(|| AppError::state("Missing retry script was not retained."))?;
    assert_eq!(sticky.kind(), FixtureErrorKind::MissingScript);
    Ok(())
}

#[test]
fn cleanup_after_complete_attempt_publication_releases_the_callback() -> AppResult<()> {
    let fixture = TranslationPolicyFixture::new();
    fixture.pause_after_attempt_publication();
    let (mut module, _outcomes) = fixture_module(&fixture)?;
    let store = CaptionAggregateStore::default();
    fixture
        .admit(
            &module,
            super::super::reservation(&store, 112, "published-before-cleanup", "private source")?,
            [AttemptScript::held_confirmed()],
        )
        .map_err(fixture_error)?;
    fixture
        .wait_until_attempt_publication_paused(FIXTURE_WATCHDOG)
        .map_err(fixture_error)?;
    let attempt = fixture
        .wait_for_attempt_count(1, FIXTURE_WATCHDOG)
        .map_err(fixture_error)?[0]
        .id();
    let attempt_probe = fixture.quiescence_probe(attempt);
    let lifetime_probe = fixture.lifetime_probe();

    std::thread::scope(|scope| -> AppResult<()> {
        let (drop_sender, drop_receiver) = std::sync::mpsc::sync_channel(1);
        let drop_thread = scope.spawn(move || {
            drop(fixture);
            let _ignored = drop_sender.send(());
        });
        lifetime_probe
            .wait_until_drop_waiting_for_adapter(FIXTURE_WATCHDOG)
            .map_err(fixture_error)?;
        assert!(matches!(
            drop_receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));

        let _proof = module.stop_and_confirm_owner_quiesced()?;
        drop_receiver
            .recv_timeout(FIXTURE_WATCHDOG)
            .map_err(|_| AppError::state("Fixture Drop did not observe owner quiescence."))?;
        drop_thread
            .join()
            .map_err(|_| AppError::state("Fixture Drop thread panicked."))?;
        Ok(())
    })?;
    attempt_probe.wait(Duration::ZERO).map_err(fixture_error)?;
    lifetime_probe.wait(Duration::ZERO).map_err(fixture_error)?;
    Ok(())
}

#[test]
fn cleanup_waits_for_a_task_paused_before_the_begin_guard() -> AppResult<()> {
    let fixture = TranslationPolicyFixture::new();
    fixture.pause_before_begin_guard();
    let (mut module, _outcomes) = fixture_module(&fixture)?;
    let store = CaptionAggregateStore::default();
    fixture
        .admit(
            &module,
            super::super::reservation(&store, 115, "paused-before-begin-guard", "private source")?,
            [AttemptScript::success("translated")],
        )
        .map_err(fixture_error)?;
    fixture
        .wait_until_before_begin_guard_paused(FIXTURE_WATCHDOG)
        .map_err(fixture_error)?;
    let lifetime_probe = fixture.lifetime_probe();

    std::thread::scope(|scope| -> AppResult<()> {
        let (drop_sender, drop_receiver) = std::sync::mpsc::sync_channel(1);
        let drop_thread = scope.spawn(move || {
            drop(fixture);
            let _ignored = drop_sender.send(());
        });
        lifetime_probe
            .wait_until_drop_waiting_for_adapter(FIXTURE_WATCHDOG)
            .map_err(fixture_error)?;
        assert!(matches!(
            drop_receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));

        let _proof = module.stop_and_confirm_owner_quiesced()?;
        drop_receiver.recv_timeout(FIXTURE_WATCHDOG).map_err(|_| {
            AppError::state("Fixture Drop returned before the Adapter task exited.")
        })?;
        drop_thread
            .join()
            .map_err(|_| AppError::state("Fixture Drop thread panicked."))?;
        Ok(())
    })?;
    lifetime_probe.wait(Duration::ZERO).map_err(fixture_error)?;
    Ok(())
}

#[test]
fn finish_waits_for_an_attempt_detached_after_owner_stop() -> AppResult<()> {
    let fixture = TranslationPolicyFixture::new();
    fixture.pause_after_attempt_publication();
    let (mut module, _outcomes) = fixture_module(&fixture)?;
    let store = CaptionAggregateStore::default();
    fixture
        .admit(
            &module,
            super::super::reservation(
                &store,
                113,
                "detached-before-begin-return",
                "private source",
            )?,
            [AttemptScript::success("translated")],
        )
        .map_err(fixture_error)?;
    fixture
        .wait_until_attempt_publication_paused(FIXTURE_WATCHDOG)
        .map_err(fixture_error)?;

    let proof = module.stop_and_confirm_owner_quiesced()?;
    let records = fixture.finish(proof).map_err(fixture_error)?;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].terminal(), Some(AttemptTerminal::Completed));
    assert!(records[0].is_quiesced());
    Ok(())
}

#[test]
fn manual_resolution_cannot_acknowledge_a_later_pre_begin_adapter_lifetime() -> AppResult<()> {
    let fixture = TranslationPolicyFixture::new();
    let (mut module, outcomes) = fixture_module(&fixture)?;
    let store = CaptionAggregateStore::default();
    fixture
        .admit(
            &module,
            super::super::reservation(
                &store,
                114,
                "manual-before-detached",
                "first private source",
            )?,
            [AttemptScript::held_confirmed()],
        )
        .map_err(fixture_error)?;
    let first = fixture
        .wait_for_attempt_count(1, FIXTURE_WATCHDOG)
        .map_err(fixture_error)?[0]
        .id();

    fixture.pause_before_begin_guard();
    fixture
        .admit(
            &module,
            super::super::reservation(
                &store,
                114,
                "detached-before-guard",
                "second private source",
            )?,
            [AttemptScript::success("second translated")],
        )
        .map_err(fixture_error)?;
    fixture
        .complete(first, "first translated")
        .map_err(fixture_error)?;
    let _first_outcome = outcomes
        .recv_timeout(FIXTURE_WATCHDOG)
        .map_err(|_| AppError::state("Manual completion produced no outcome."))?;
    fixture
        .wait_until_before_begin_guard_paused(FIXTURE_WATCHDOG)
        .map_err(fixture_error)?;
    let proof = module.stop_and_confirm_owner_quiesced()?;

    std::thread::scope(|scope| -> AppResult<()> {
        let (finished_sender, finished_receiver) = std::sync::mpsc::sync_channel(1);
        let fixture_ref = &fixture;
        let finish_thread = scope.spawn(move || {
            let result = fixture_ref.finish(proof);
            let _ignored = finished_sender.send(result);
        });
        fixture
            .wait_until_finish_waiting_for_adapter(FIXTURE_WATCHDOG)
            .map_err(fixture_error)?;
        assert!(matches!(
            finished_receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));

        fixture.release_before_begin_guard();
        let error = finished_receiver
            .recv_timeout(FIXTURE_WATCHDOG)
            .map_err(|_| AppError::state("Finish did not observe the real Adapter Drop."))?
            .err()
            .ok_or_else(|| AppError::state("Unused detached-attempt script was accepted."))?;
        assert_eq!(error.kind(), FixtureErrorKind::UnusedScripts);
        finish_thread
            .join()
            .map_err(|_| AppError::state("Finish observer thread panicked."))?;
        Ok(())
    })?;
    Ok(())
}

#[test]
fn wrong_phase_quiescence_is_sticky_and_cleanup_still_quiesces() -> AppResult<()> {
    let fixture = TranslationPolicyFixture::new();
    let (mut module, _outcomes) = fixture_module(&fixture)?;
    let store = CaptionAggregateStore::default();
    fixture
        .admit(
            &module,
            super::super::reservation(&store, 101, "wrong-phase", "private source")?,
            [AttemptScript::held_confirmed()],
        )
        .map_err(fixture_error)?;
    let attempt = fixture
        .wait_for_attempt_count(1, FIXTURE_WATCHDOG)
        .map_err(fixture_error)?[0]
        .id();
    let probe = fixture.quiescence_probe(attempt);

    let error = fixture
        .quiesce_unconfirmed(attempt)
        .err()
        .ok_or_else(|| AppError::state("Wrong-phase quiescence was accepted."))?;
    assert_eq!(error.kind(), FixtureErrorKind::WrongAttemptPhase);
    let owner = module.stop_and_confirm_owner_quiesced()?;
    probe.wait(FIXTURE_WATCHDOG).map_err(fixture_error)?;
    let sticky = fixture
        .finish(owner)
        .err()
        .ok_or_else(|| AppError::state("Fixture misuse was not retained."))?;
    assert_eq!(sticky.kind(), FixtureErrorKind::WrongAttemptPhase);
    Ok(())
}

#[test]
fn duplicate_resolution_is_sticky() -> AppResult<()> {
    let fixture = TranslationPolicyFixture::new();
    let (mut module, outcomes) = fixture_module(&fixture)?;
    let store = CaptionAggregateStore::default();
    fixture
        .admit(
            &module,
            super::super::reservation(&store, 102, "duplicate-resolve", "private source")?,
            [AttemptScript::held_confirmed()],
        )
        .map_err(fixture_error)?;
    let attempt = fixture
        .wait_for_attempt_count(1, FIXTURE_WATCHDOG)
        .map_err(fixture_error)?[0]
        .id();
    fixture
        .complete(attempt, "translated")
        .map_err(fixture_error)?;
    let _outcome = outcomes
        .recv_timeout(FIXTURE_WATCHDOG)
        .map_err(|_| AppError::state("Resolved attempt produced no outcome."))?;
    fixture
        .wait_for_quiescence(attempt, FIXTURE_WATCHDOG)
        .map_err(fixture_error)?;

    let error = fixture
        .complete(attempt, "duplicate")
        .err()
        .ok_or_else(|| AppError::state("Duplicate resolution was accepted."))?;
    assert_eq!(error.kind(), FixtureErrorKind::WrongAttemptPhase);
    let owner = module.stop_and_confirm_owner_quiesced()?;
    let sticky = fixture
        .finish(owner)
        .err()
        .ok_or_else(|| AppError::state("Duplicate resolution was not retained."))?;
    assert_eq!(sticky.kind(), FixtureErrorKind::WrongAttemptPhase);
    Ok(())
}

#[test]
fn duplicate_cancellation_is_sticky() -> AppResult<()> {
    let fixture = TranslationPolicyFixture::new();
    let (mut module, outcomes) = fixture_module(&fixture)?;
    let store = CaptionAggregateStore::default();
    fixture
        .admit(
            &module,
            super::super::reservation(&store, 103, "duplicate-cancel", "private source")?,
            [AttemptScript::held_unconfirmed()],
        )
        .map_err(fixture_error)?;
    let attempt = fixture
        .wait_for_attempt_count(1, FIXTURE_WATCHDOG)
        .map_err(fixture_error)?[0]
        .id();
    fixture.advance(TOTAL_DEADLINE).map_err(fixture_error)?;
    let _outcome = outcomes
        .recv_timeout(FIXTURE_WATCHDOG)
        .map_err(|_| AppError::state("Cancelled attempt produced no outcome."))?;
    let record = fixture
        .wait_for_cancellation(attempt, CancellationStatus::Unconfirmed, FIXTURE_WATCHDOG)
        .map_err(fixture_error)?;
    assert_eq!(
        record.terminal(),
        Some(AttemptTerminal::CancelledUnconfirmed)
    );
    assert_eq!(record.cancellation(), Some(CancellationStatus::Unconfirmed));
    fixture
        .quiesce_unconfirmed(attempt)
        .map_err(fixture_error)?;

    let mut active = FixtureActiveCall::new(attempt, Arc::clone(&fixture.shared));
    assert_eq!(active.cancel(), CancellationStatus::Unconfirmed);
    drop(active);
    let owner = module.stop_and_confirm_owner_quiesced()?;
    let sticky = fixture
        .finish(owner)
        .err()
        .ok_or_else(|| AppError::state("Duplicate cancellation was not retained."))?;
    assert_eq!(sticky.kind(), FixtureErrorKind::WrongAttemptPhase);
    Ok(())
}

#[test]
fn duplicate_non_cooperative_release_is_sticky() -> AppResult<()> {
    let fixture = TranslationPolicyFixture::new();
    let (mut module, outcomes) = fixture_module(&fixture)?;
    let store = CaptionAggregateStore::default();
    fixture
        .admit(
            &module,
            super::super::reservation(&store, 104, "duplicate-release", "private source")?,
            [AttemptScript::non_cooperative_success("translated")],
        )
        .map_err(fixture_error)?;
    let attempt = fixture
        .wait_for_attempt_count(1, FIXTURE_WATCHDOG)
        .map_err(fixture_error)?[0]
        .id();
    fixture
        .release_non_cooperative(attempt)
        .map_err(fixture_error)?;
    fixture
        .wait_for_quiescence(attempt, FIXTURE_WATCHDOG)
        .map_err(fixture_error)?;
    let _outcome = outcomes
        .recv_timeout(FIXTURE_WATCHDOG)
        .map_err(|_| AppError::state("Released attempt produced no outcome."))?;

    let error = fixture
        .release_non_cooperative(attempt)
        .err()
        .ok_or_else(|| AppError::state("Duplicate release was accepted."))?;
    assert_eq!(error.kind(), FixtureErrorKind::WrongAttemptPhase);
    let owner = module.stop_and_confirm_owner_quiesced()?;
    let sticky = fixture
        .finish(owner)
        .err()
        .ok_or_else(|| AppError::state("Duplicate release was not retained."))?;
    assert_eq!(sticky.kind(), FixtureErrorKind::WrongAttemptPhase);
    Ok(())
}

#[test]
fn finish_rejects_an_active_logical_delay() -> AppResult<()> {
    let fixture = TranslationPolicyFixture::new();
    let (mut module, _outcomes) = fixture_module(&fixture)?;
    let store = CaptionAggregateStore::default();
    fixture
        .admit(
            &module,
            super::super::reservation(&store, 105, "active-delay", "private source")?,
            [AttemptScript::failure(
                TranslationFailureClass::ServiceUnavailable,
                true,
                None,
                false,
            )],
        )
        .map_err(fixture_error)?;
    fixture
        .wait_for_delay_count(1, FIXTURE_WATCHDOG)
        .map_err(fixture_error)?;

    let error = fixture
        .validate_quiescence()
        .err()
        .ok_or_else(|| AppError::state("Active delay was accepted as finished."))?;
    assert_eq!(error.kind(), FixtureErrorKind::DelaysStillActive);
    let _owner = module.stop_and_confirm_owner_quiesced()?;
    Ok(())
}

#[test]
fn successful_finish_is_one_shot_and_closes_mutations() -> AppResult<()> {
    let fixture = TranslationPolicyFixture::new();
    let (mut module, _outcomes) = fixture_module(&fixture)?;
    let owner = module.stop_and_confirm_owner_quiesced()?;
    assert!(fixture.finish(owner).map_err(fixture_error)?.is_empty());

    let advance = fixture
        .advance(Duration::from_millis(1))
        .err()
        .ok_or_else(|| AppError::state("Finished fixture accepted a time advance."))?;
    assert_eq!(advance.kind(), FixtureErrorKind::AlreadyFinished);
    let store = CaptionAggregateStore::default();
    let admission = fixture
        .admit(
            &module,
            super::super::reservation(&store, 107, "after-finish", "private source")?,
            [AttemptScript::success("translated")],
        )
        .err()
        .ok_or_else(|| AppError::state("Finished fixture accepted an admission."))?;
    assert_eq!(admission.kind(), FixtureErrorKind::AlreadyFinished);
    let duplicate_owner = module.stop_and_confirm_owner_quiesced()?;
    let duplicate = fixture
        .finish(duplicate_owner)
        .err()
        .ok_or_else(|| AppError::state("Fixture finish was accepted twice."))?;
    assert_eq!(duplicate.kind(), FixtureErrorKind::AlreadyFinished);
    Ok(())
}

#[test]
fn cleanup_closes_a_begin_paused_after_its_inflight_guard() -> AppResult<()> {
    let probe = Arc::new(Mutex::new(None));
    let captured_probe = Arc::clone(&probe);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let fixture = TranslationPolicyFixture::new();
        fixture.pause_after_begin_guard();
        let (module, _outcomes) = fixture_module(&fixture)
            .unwrap_or_else(|error| std::panic::resume_unwind(Box::new(error)));
        let store = CaptionAggregateStore::default();
        fixture
            .admit(
                &module,
                super::super::reservation(&store, 108, "guard-paused", "private source")
                    .unwrap_or_else(|error| std::panic::resume_unwind(Box::new(error))),
                [AttemptScript::held_confirmed()],
            )
            .unwrap_or_else(|error| std::panic::resume_unwind(Box::new(error)));
        fixture
            .wait_until_begin_guard_paused(FIXTURE_WATCHDOG)
            .unwrap_or_else(|error| std::panic::resume_unwind(Box::new(error)));
        *captured_probe
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(fixture.lifetime_probe());
        std::panic::resume_unwind(Box::new("original guard-paused failure"));
    }));
    let payload = panic
        .err()
        .ok_or_else(|| AppError::state("Guard-paused unwind did not panic."))?;
    assert_eq!(
        panic_text(payload.as_ref()),
        Some("original guard-paused failure")
    );
    let probe = probe
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
        .ok_or_else(|| AppError::state("Guard-paused scenario produced no probe."))?;
    probe.wait(Duration::ZERO).map_err(fixture_error)?;
    Ok(())
}

#[test]
fn panicking_script_iterator_cannot_poison_blocker_cleanup() -> AppResult<()> {
    let fixture = TranslationPolicyFixture::new();
    let (mut module, outcomes) = fixture_module(&fixture)?;
    let store = CaptionAggregateStore::default();
    fixture
        .admit(
            &module,
            super::super::reservation(
                &store,
                109,
                "blocked-before-iterator",
                "first private source",
            )?,
            [AttemptScript::non_cooperative_success("translated")],
        )
        .map_err(fixture_error)?;
    let attempt = fixture
        .wait_for_attempt_count(1, FIXTURE_WATCHDOG)
        .map_err(fixture_error)?[0]
        .id();

    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        fixture
            .admit(
                &module,
                super::super::reservation(
                    &store,
                    109,
                    "panicking-iterator",
                    "second private source",
                )
                .unwrap_or_else(|error| std::panic::resume_unwind(Box::new(error))),
                PanickingScripts,
            )
            .unwrap_or_else(|error| std::panic::resume_unwind(Box::new(error)));
    }));
    let payload = panic
        .err()
        .ok_or_else(|| AppError::state("Script iterator did not panic."))?;
    assert_eq!(
        panic_text(payload.as_ref()),
        Some("original script iterator failure")
    );

    fixture
        .release_non_cooperative(attempt)
        .map_err(fixture_error)?;
    fixture
        .wait_for_quiescence(attempt, FIXTURE_WATCHDOG)
        .map_err(fixture_error)?;
    let _outcome = outcomes
        .recv_timeout(FIXTURE_WATCHDOG)
        .map_err(|_| AppError::state("Released blocker produced no outcome."))?;
    let owner = module.stop_and_confirm_owner_quiesced()?;
    assert_eq!(fixture.finish(owner).map_err(fixture_error)?.len(), 1);
    Ok(())
}

#[test]
fn unwinding_drop_releases_and_waits_for_non_cooperative_begin() -> AppResult<()> {
    let probe = Arc::new(Mutex::new(None));
    let captured_probe = Arc::clone(&probe);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let fixture = TranslationPolicyFixture::new();
        // Force poisoning after publication but before the branch re-locks
        // state to own its blocker; merely seeing an attempt left this racing.
        fixture.pause_after_attempt_publication();
        let (module, _outcomes) = fixture_module(&fixture)
            .unwrap_or_else(|error| std::panic::resume_unwind(Box::new(error)));
        let store = CaptionAggregateStore::default();
        fixture
            .admit(
                &module,
                super::super::reservation(&store, 106, "unwind-non-cooperative", "private source")
                    .unwrap_or_else(|error| std::panic::resume_unwind(Box::new(error))),
                [AttemptScript::non_cooperative_success("late translation")],
            )
            .unwrap_or_else(|error| std::panic::resume_unwind(Box::new(error)));
        let attempt = fixture
            .wait_for_attempt_count(1, FIXTURE_WATCHDOG)
            .unwrap_or_else(|error| std::panic::resume_unwind(Box::new(error)))[0]
            .id();
        fixture
            .wait_until_attempt_publication_paused(FIXTURE_WATCHDOG)
            .unwrap_or_else(|error| std::panic::resume_unwind(Box::new(error)));
        *captured_probe
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(fixture.quiescence_probe(attempt));
        let _poison = fixture
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::panic::resume_unwind(Box::new("original scripted failure"));
    }));
    let payload = panic
        .err()
        .ok_or_else(|| AppError::state("Non-cooperative unwind did not panic."))?;
    assert_eq!(
        panic_text(payload.as_ref()),
        Some("original scripted failure")
    );
    let probe = probe
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
        .ok_or_else(|| AppError::state("Unwind scenario produced no quiescence probe."))?;
    probe.wait(Duration::ZERO).map_err(fixture_error)?;
    let state = probe
        .shared
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let attempt = state
        .attempts
        .get(&probe.id)
        .ok_or_else(|| AppError::state("Unwind scenario lost its recorded attempt."))?;
    assert!(attempt.completion.is_none());
    assert!(attempt.non_cooperative_resolution.is_none());
    Ok(())
}
