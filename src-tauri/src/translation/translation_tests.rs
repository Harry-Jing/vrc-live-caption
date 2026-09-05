use super::*;
use crate::caption::{CaptionAggregateStore, CaptionLane, CaptionSnapshot, CaptionState};
use crate::config::{ApiBaseUrl, TranslationConfig, TranslationEndpoint, TranslationPath};
use crate::credentials::{CredentialId, CredentialStorage, ResolvedCredential};
use crate::error::{AppError, AppResult};
use crate::host_resolver::HostResolver;
use secrecy::SecretString;
use std::time::Duration;

#[path = "policy_test_fixture.rs"]
mod fixture;

use fixture::{AttemptScript, FixtureModule, TranslationPolicyFixture};

const TEST_TIMEOUT: Duration = Duration::from_secs(1);

fn fixture_error(error: fixture::FixtureError) -> AppError {
    AppError::state(error.to_string())
}

fn stop_and_finish_fixture(
    fixture: &TranslationPolicyFixture,
    module: &mut FixtureModule,
) -> AppResult<Vec<fixture::AttemptRecord>> {
    let owner = module.stop_and_confirm_owner_quiesced()?;
    fixture.finish(owner).map_err(fixture_error)
}

fn distinct_source(byte_len: usize, discriminator: u8) -> String {
    let mut source = "x".repeat(byte_len.max(1));
    source.replace_range(
        ..1,
        &char::from(b'a'.saturating_add(discriminator % 26)).to_string(),
    );
    source
}

fn reservation(
    store: &CaptionAggregateStore,
    generation: u64,
    unit: &str,
    text: impl Into<String>,
) -> AppResult<crate::caption::ReservedCompletedSource> {
    let active = store
        .begin_generation(generation)?
        .active_stream
        .ok_or_else(|| {
            AppError::state("Test generation did not produce an active caption stream.")
        })?;
    store.start_unit(generation, &active.stream_id, unit.to_string(), 10)?;
    let caption = CaptionSnapshot {
        generation,
        stream_id: active.stream_id,
        unit_id: Some(unit.to_string()),
        lane: CaptionLane::Source,
        revision: 1,
        text: text.into(),
        state: CaptionState::Completed,
        language: Some("en".to_string()),
        source_ref: None,
        unit_started_at_ms: Some(10),
        timestamp_ms: 20,
    };
    store
        .accept_completed_source_for_translation(caption)?
        .map(|(_, reservation)| reservation)
        .ok_or_else(|| AppError::state("Test source was not reserved."))
}

fn accept_completed_source(
    store: &CaptionAggregateStore,
    generation: u64,
    unit: &str,
    text: impl Into<String>,
) -> AppResult<CaptionSnapshot> {
    let active = store
        .begin_generation(generation)?
        .active_stream
        .ok_or_else(|| AppError::state("Test generation did not produce an active stream."))?;
    store.start_unit(generation, &active.stream_id, unit.to_string(), 10)?;
    let caption = CaptionSnapshot {
        generation,
        stream_id: active.stream_id,
        unit_id: Some(unit.to_string()),
        lane: CaptionLane::Source,
        revision: 1,
        text: text.into(),
        state: CaptionState::Completed,
        language: Some("en".to_string()),
        source_ref: None,
        unit_started_at_ms: Some(10),
        timestamp_ms: 20,
    };
    store.accept_caption(caption.clone())?;
    Ok(caption)
}

fn translation_credential(id: CredentialId) -> ResolvedCredential {
    ResolvedCredential {
        id,
        secret: SecretString::from("test-translation-secret"),
        storage: CredentialStorage::SystemCredentialStore,
        display_suffix: Some("test".to_string()),
    }
}

#[test]
fn production_factory_binds_each_endpoint_to_its_own_credential() -> AppResult<()> {
    let custom_base =
        ApiBaseUrl::parse("https://translation.example.test/api/v1").map_err(AppError::config)?;
    let cases = [
        (
            TranslationEndpoint::Official,
            CredentialId::OpenAi,
            CredentialId::CustomTranslation,
        ),
        (
            TranslationEndpoint::Custom {
                api_base_url: custom_base,
            },
            CredentialId::CustomTranslation,
            CredentialId::OpenAi,
        ),
    ];

    for (endpoint, expected, mismatched) in cases {
        let selection = TranslationConfig {
            path: TranslationPath::OpenAiResponsesCompletedText,
            target: TranslationTarget::English,
            endpoint,
        };
        let binding = openai_responses_completed_text_module(
            selection.clone(),
            translation_credential(expected),
            1,
            HostResolver::default(),
        )?;
        binding.stop_for_test()?;

        let error = openai_responses_completed_text_module(
            selection,
            translation_credential(mismatched),
            1,
            HostResolver::default(),
        )
        .err()
        .ok_or_else(|| AppError::state("A mismatched Translation credential was accepted."))?;
        assert_eq!(error.code(), "runtime.failed");
    }
    Ok(())
}

#[test]
fn completed_work_is_fifo_and_correlation_stays_owned_by_the_module() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let fixture = TranslationPolicyFixture::new();
    let (mut module, outcomes) = fixture
        .start_module(TranslationTarget::SimplifiedChinese)
        .map_err(fixture_error)?;
    assert!(matches!(
        outcomes.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));

    fixture
        .admit(
            &module,
            reservation(&store, 1, "one", "first private source")?,
            [AttemptScript::held_confirmed()],
        )
        .map_err(fixture_error)?;
    let first_attempt = fixture
        .wait_for_attempt_count(1, TEST_TIMEOUT)
        .map_err(fixture_error)?[0]
        .id();
    let second = reservation(&store, 1, "two", "repeated private source")?;
    let third = reservation(&store, 1, "three", "repeated private source")?;
    fixture
        .admit(&module, second, [AttemptScript::success("two translated")])
        .map_err(fixture_error)?;
    fixture
        .admit(&module, third, [AttemptScript::success("three translated")])
        .map_err(fixture_error)?;
    fixture
        .complete(first_attempt, "one translated")
        .map_err(fixture_error)?;

    let first_outcome = outcomes
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|_| AppError::state("First terminal translation outcome was not received."))?;
    let second_outcome = outcomes
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|_| AppError::state("Second terminal translation outcome was not received."))?;
    let third_outcome = outcomes
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|_| AppError::state("Third terminal translation outcome was not received."))?;
    assert_eq!(first_outcome.source_ref().unit_id, "one");
    assert_eq!(second_outcome.source_ref().unit_id, "two");
    assert_eq!(third_outcome.source_ref().unit_id, "three");
    let first_ref = first_outcome.source_ref().clone();
    let second_ref = second_outcome.source_ref().clone();
    let third_ref = third_outcome.source_ref().clone();
    let TranslationTerminalOutcome::Completed(first) = first_outcome else {
        return Err(AppError::state("First translation was not completed."));
    };
    let TranslationTerminalOutcome::Completed(second) = second_outcome else {
        return Err(AppError::state("Second translation was not completed."));
    };
    let TranslationTerminalOutcome::Completed(third) = third_outcome else {
        return Err(AppError::state("Third translation was not completed."));
    };
    let first_update = first
        .complete(30)?
        .ok_or_else(|| AppError::state("First translation was not finalized."))?;
    let second_update = second
        .complete(31)?
        .ok_or_else(|| AppError::state("Second translation was not finalized."))?;
    let third_update = third
        .complete(32)?
        .ok_or_else(|| AppError::state("Third translation was not finalized."))?;
    let snapshots = [first_update, second_update, third_update].map(|update| match update.change {
        crate::caption::CaptionAggregateChange::CaptionAccepted(snapshot) => Ok(snapshot),
        _ => Err(AppError::state(
            "Completed translation did not produce a caption snapshot.",
        )),
    });
    let [first_snapshot, second_snapshot, third_snapshot] = snapshots;
    let first_snapshot = first_snapshot?;
    let second_snapshot = second_snapshot?;
    let third_snapshot = third_snapshot?;
    assert_eq!(first_snapshot.lane, CaptionLane::Translation);
    assert_eq!(second_snapshot.lane, CaptionLane::Translation);
    assert_eq!(third_snapshot.lane, CaptionLane::Translation);
    assert_eq!(first_snapshot.text, "one translated");
    assert_eq!(second_snapshot.text, "two translated");
    assert_eq!(third_snapshot.text, "three translated");
    assert_eq!(first_snapshot.unit_id.as_deref(), Some("one"));
    assert_eq!(second_snapshot.unit_id.as_deref(), Some("two"));
    assert_eq!(third_snapshot.unit_id.as_deref(), Some("three"));

    let records = stop_and_finish_fixture(&fixture, &mut module)?;
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].source_ref().unit_id, "one");
    assert_eq!(records[1].source_ref().unit_id, "two");
    assert_eq!(records[2].source_ref().unit_id, "three");
    assert_eq!(records[0].source_ref(), &first_ref);
    assert_eq!(records[1].source_ref(), &second_ref);
    assert_eq!(records[2].source_ref(), &third_ref);
    assert_ne!(records[1].source_id(), records[2].source_id());
    assert_eq!(records[0].target(), TranslationTarget::SimplifiedChinese);
    assert_eq!(records[0].attempt_number(), 1);
    Ok(())
}

#[test]
fn admission_enforces_count_and_source_byte_budgets_without_waiting() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let fixture = TranslationPolicyFixture::new();
    let (mut module, _outcomes) = fixture
        .start_module(TranslationTarget::SimplifiedChinese)
        .map_err(fixture_error)?;

    for index in 0_u64..8 {
        fixture
            .admit(
                &module,
                reservation(
                    &store,
                    2,
                    &format!("unit-{index}"),
                    distinct_source(8 * 1024, u8::try_from(index).unwrap_or_default()),
                )?,
                (index == 0).then_some(AttemptScript::held_confirmed()),
            )
            .map_err(fixture_error)?;
    }
    let active = fixture
        .wait_for_attempt_count(1, TEST_TIMEOUT)
        .map_err(fixture_error)?[0]
        .id();
    assert_eq!(
        module
            .try_submit(reservation(&store, 2, "ninth", "x")?)
            .map_err(|rejection| rejection.kind()),
        Err(TranslationSubmitError::OutstandingLimit)
    );
    assert_eq!(
        module
            .try_submit(reservation(
                &store,
                2,
                "oversized",
                "x".repeat(16 * 1024 + 1),
            )?)
            .map_err(|rejection| rejection.kind()),
        Err(TranslationSubmitError::SourceTooLarge)
    );

    let owner = module.stop_and_confirm_owner_quiesced()?;
    fixture
        .wait_for_quiescence(active, TEST_TIMEOUT)
        .map_err(fixture_error)?;
    fixture.finish(owner).map_err(fixture_error)?;
    Ok(())
}

#[test]
fn rejected_submission_retains_the_one_shot_failure_capability() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let fixture = TranslationPolicyFixture::new();
    let (mut module, _outcomes) = fixture
        .start_module(TranslationTarget::SimplifiedChinese)
        .map_err(fixture_error)?;
    for index in 0_u64..OUTSTANDING_LIMIT as u64 {
        fixture
            .admit(
                &module,
                reservation(
                    &store,
                    31,
                    &format!("admitted-{index}"),
                    format!("admitted source {index}"),
                )?,
                (index == 0).then_some(AttemptScript::held_confirmed()),
            )
            .map_err(fixture_error)?;
    }
    let active = fixture
        .wait_for_attempt_count(1, TEST_TIMEOUT)
        .map_err(fixture_error)?[0]
        .id();

    let rejection = module
        .try_submit(reservation(&store, 31, "rejected", "rejected source")?)
        .err()
        .ok_or_else(|| AppError::state("Over-budget source was unexpectedly admitted."))?;
    assert_eq!(rejection.kind(), TranslationSubmitError::OutstandingLimit);
    assert_eq!(
        rejection.reason(),
        crate::caption::TranslationFailureReason::Backpressure
    );
    assert!(!format!("{rejection:?}").contains("rejected source"));
    assert!(store.snapshot()?.translation_units.iter().any(|unit| {
        matches!(
            unit,
            crate::caption::TranslationUnitSnapshot::Pending { source_ref }
                if source_ref.unit_id == "rejected"
        )
    }));

    let update = rejection
        .fail()?
        .ok_or_else(|| AppError::state("Rejected Translation was not terminalized."))?;
    assert!(matches!(
        update.change,
        crate::caption::CaptionAggregateChange::TranslationFailed(
            crate::caption::TranslationUnitSnapshot::Failed {
                reason_code: crate::caption::TranslationFailureReason::Backpressure,
                ..
            }
        )
    ));
    let owner = module.stop_and_confirm_owner_quiesced()?;
    fixture
        .wait_for_quiescence(active, TEST_TIMEOUT)
        .map_err(fixture_error)?;
    fixture.finish(owner).map_err(fixture_error)?;
    Ok(())
}

#[test]
fn completed_outcomes_hold_source_bytes_until_finalized_or_dropped() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let fixture = TranslationPolicyFixture::new();
    let (mut module, outcomes) = fixture
        .start_module(TranslationTarget::SimplifiedChinese)
        .map_err(fixture_error)?;
    for index in 0_u64..4 {
        fixture
            .admit(
                &module,
                reservation(
                    &store,
                    13,
                    &format!("held-{index}"),
                    distinct_source(SOURCE_BYTE_LIMIT, u8::try_from(index).unwrap_or_default()),
                )?,
                [AttemptScript::success(format!("translated-{index}"))],
            )
            .map_err(fixture_error)?;
    }
    let mut held = Vec::new();
    for _ in 0..4 {
        held.push(
            outcomes
                .recv_timeout(TEST_TIMEOUT)
                .map_err(|_| AppError::state("Held completed outcome was not received."))?,
        );
    }

    assert_eq!(
        module
            .try_submit(reservation(
                &store,
                13,
                "retained-limit",
                "x".repeat(SOURCE_BYTE_LIMIT),
            )?)
            .map_err(|rejection| rejection.kind()),
        Err(TranslationSubmitError::RetainedSourceLimit)
    );
    drop(held.pop());
    fixture
        .admit(
            &module,
            reservation(
                &store,
                13,
                "after-release",
                distinct_source(SOURCE_BYTE_LIMIT, 5),
            )?,
            [AttemptScript::success("translated-after-release")],
        )
        .map_err(fixture_error)?;
    fixture
        .wait_for_attempt_count(5, TEST_TIMEOUT)
        .map_err(fixture_error)?;
    stop_and_finish_fixture(&fixture, &mut module)?;
    Ok(())
}

#[test]
fn unconsumed_outcomes_continue_to_hold_the_outstanding_slots() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let fixture = TranslationPolicyFixture::new();
    let (mut module, _outcomes) = fixture
        .start_module(TranslationTarget::SimplifiedChinese)
        .map_err(fixture_error)?;
    for index in 0_u64..8 {
        fixture
            .admit(
                &module,
                reservation(
                    &store,
                    14,
                    &format!("outcome-{index}"),
                    format!("source-{index}"),
                )?,
                [AttemptScript::success(format!("translated-{index}"))],
            )
            .map_err(fixture_error)?;
    }
    fixture
        .wait_for_attempt_count(8, TEST_TIMEOUT)
        .map_err(fixture_error)?;
    assert_eq!(
        module
            .try_submit(reservation(&store, 14, "ninth", "x")?)
            .map_err(|rejection| rejection.kind()),
        Err(TranslationSubmitError::OutstandingLimit)
    );
    stop_and_finish_fixture(&fixture, &mut module)?;
    Ok(())
}

#[test]
fn stop_releases_active_and_queued_reservations_and_suppresses_late_results() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let fixture = TranslationPolicyFixture::new();
    let (mut module, outcomes) = fixture
        .start_module(TranslationTarget::SimplifiedChinese)
        .map_err(fixture_error)?;
    fixture
        .admit(
            &module,
            reservation(&store, 3, "active", "active private source")?,
            [AttemptScript::held_confirmed()],
        )
        .map_err(fixture_error)?;
    let active = fixture
        .wait_for_attempt_count(1, TEST_TIMEOUT)
        .map_err(fixture_error)?[0]
        .id();
    fixture
        .admit(
            &module,
            reservation(&store, 3, "queued", "queued private source")?,
            [],
        )
        .map_err(fixture_error)?;

    let owner = module.stop_and_confirm_owner_quiesced()?;
    fixture
        .wait_for_quiescence(active, TEST_TIMEOUT)
        .map_err(fixture_error)?;
    fixture.finish(owner).map_err(fixture_error)?;
    assert!(outcomes.recv_timeout(Duration::from_millis(50)).is_err());
    assert_eq!(
        module
            .try_submit(reservation(&store, 3, "late", "late source")?)
            .map_err(|rejection| rejection.kind()),
        Err(TranslationSubmitError::Stopped)
    );

    Ok(())
}

#[test]
fn stop_does_not_wait_for_a_non_cooperative_adapter_to_return() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let fixture = TranslationPolicyFixture::new();
    let (mut module, outcomes) = fixture
        .start_module(TranslationTarget::SimplifiedChinese)
        .map_err(fixture_error)?;
    fixture
        .admit(
            &module,
            reservation(&store, 4, "non-cooperative", "private source")?,
            [AttemptScript::non_cooperative_success(
                "late private translation",
            )],
        )
        .map_err(fixture_error)?;
    let attempt = fixture
        .wait_for_attempt_count(1, TEST_TIMEOUT)
        .map_err(fixture_error)?[0]
        .id();

    let (stopped_sender, stopped_receiver) = std::sync::mpsc::sync_channel(1);
    let owner = std::thread::scope(|scope| -> AppResult<TranslationOwnerQuiesced> {
        let stop_task = scope.spawn(move || {
            let result = module.stop_and_confirm_owner_quiesced();
            let _ignored = stopped_sender.send(());
            result
        });
        if stopped_receiver.recv_timeout(TEST_TIMEOUT).is_err() {
            fixture
                .release_non_cooperative(attempt)
                .map_err(fixture_error)?;
            let _ignored = stop_task.join();
            return Err(AppError::state(
                "Translation Stop did not return before the fixture watchdog.",
            ));
        }
        let record = fixture
            .wait_for_attempt_count(1, TEST_TIMEOUT)
            .map_err(fixture_error)?
            .into_iter()
            .next()
            .ok_or_else(|| AppError::state("Non-cooperative attempt was not recorded."))?;
        assert!(!record.is_quiesced());
        fixture
            .release_non_cooperative(attempt)
            .map_err(fixture_error)?;
        fixture
            .wait_for_quiescence(attempt, TEST_TIMEOUT)
            .map_err(fixture_error)?;
        let owner = stop_task
            .join()
            .map_err(|_| AppError::state("Translation Stop test thread panicked."))??;
        Ok(owner)
    })?;

    fixture.finish(owner).map_err(fixture_error)?;
    assert!(outcomes.recv_timeout(Duration::from_millis(50)).is_err());

    Ok(())
}

#[test]
fn retryable_failure_uses_two_attempts_and_the_deterministic_backoff() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let fixture = TranslationPolicyFixture::new();
    let (mut module, outcomes) = fixture
        .start_module_with_jitter(
            TranslationTarget::SimplifiedChinese,
            Duration::from_millis(250),
        )
        .map_err(fixture_error)?;
    fixture
        .admit(
            &module,
            reservation(&store, 5, "retry", "private source")?,
            [
                AttemptScript::failure(
                    TranslationFailureClass::ServiceUnavailable,
                    true,
                    None,
                    false,
                ),
                AttemptScript::success("translated"),
            ],
        )
        .map_err(fixture_error)?;
    fixture
        .wait_for_attempt_count(1, TEST_TIMEOUT)
        .map_err(fixture_error)?;
    let delays = fixture
        .wait_for_delay_count(1, TEST_TIMEOUT)
        .map_err(fixture_error)?;
    assert_eq!(delays[0].requested(), Duration::from_millis(250));
    assert_eq!(delays[0].started_at(), Duration::ZERO);
    assert_eq!(delays[0].finished_at(), None);
    fixture
        .advance(Duration::from_millis(250))
        .map_err(fixture_error)?;

    assert!(matches!(
        outcomes.recv_timeout(TEST_TIMEOUT),
        Ok(TranslationTerminalOutcome::Completed(_))
    ));
    let records = stop_and_finish_fixture(&fixture, &mut module)?;
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].source_id(), records[1].source_id());
    assert_eq!(records[0].attempt_number(), 1);
    assert_eq!(records[1].attempt_number(), 2);
    assert_eq!(records[0].finished_at(), Some(Duration::ZERO));
    assert_eq!(records[1].finished_at(), Some(Duration::from_millis(250)));
    assert_eq!(records[0].target(), TranslationTarget::SimplifiedChinese);
    assert_eq!(records[1].target(), TranslationTarget::SimplifiedChinese);
    assert_eq!(
        (records[0].attempt_budget(), records[0].total_budget()),
        (Duration::from_secs(5), Duration::from_secs(12))
    );
    assert_eq!(
        (records[1].attempt_budget(), records[1].total_budget()),
        (Duration::from_secs(5), Duration::from_millis(11_750))
    );
    Ok(())
}

#[test]
fn confirmed_attempt_timeout_cancels_before_retrying() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let fixture = TranslationPolicyFixture::new();
    let (mut module, outcomes) = fixture
        .start_module_with_jitter(
            TranslationTarget::SimplifiedChinese,
            Duration::from_millis(250),
        )
        .map_err(fixture_error)?;
    fixture
        .admit(
            &module,
            reservation(&store, 24, "attempt-deadline", "private source")?,
            [
                AttemptScript::held_confirmed(),
                AttemptScript::held_confirmed(),
            ],
        )
        .map_err(fixture_error)?;
    let first = fixture
        .wait_for_attempt_count(1, TEST_TIMEOUT)
        .map_err(fixture_error)?[0]
        .id();

    fixture.advance(ATTEMPT_DEADLINE).map_err(fixture_error)?;
    let delays = fixture
        .wait_for_delay_count(1, TEST_TIMEOUT)
        .map_err(fixture_error)?;
    assert_eq!(delays[0].requested(), Duration::from_millis(250));
    let cancelled = fixture
        .wait_for_quiescence(first, TEST_TIMEOUT)
        .map_err(fixture_error)?;
    assert_eq!(
        cancelled.terminal(),
        Some(fixture::AttemptTerminal::CancelledConfirmed)
    );
    fixture
        .advance(Duration::from_millis(250))
        .map_err(fixture_error)?;
    let second = fixture
        .wait_for_attempt_count(2, TEST_TIMEOUT)
        .map_err(fixture_error)?[1]
        .id();
    fixture
        .complete(second, "translated")
        .map_err(fixture_error)?;
    assert!(matches!(
        outcomes.recv_timeout(TEST_TIMEOUT),
        Ok(TranslationTerminalOutcome::Completed(_))
    ));
    assert_eq!(stop_and_finish_fixture(&fixture, &mut module)?.len(), 2);
    Ok(())
}

#[test]
fn stop_cancels_retry_backoff_before_another_call_starts() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let fixture = TranslationPolicyFixture::new();
    let (mut module, outcomes) = fixture
        .start_module_with_jitter(
            TranslationTarget::SimplifiedChinese,
            Duration::from_millis(250),
        )
        .map_err(fixture_error)?;
    fixture
        .admit(
            &module,
            reservation(&store, 25, "stop-backoff", "private source")?,
            [AttemptScript::failure(
                TranslationFailureClass::ServiceUnavailable,
                true,
                None,
                false,
            )],
        )
        .map_err(fixture_error)?;
    fixture
        .wait_for_delay_count(1, TEST_TIMEOUT)
        .map_err(fixture_error)?;

    assert_eq!(stop_and_finish_fixture(&fixture, &mut module)?.len(), 1);
    assert!(outcomes.recv_timeout(Duration::from_millis(50)).is_err());
    Ok(())
}

#[test]
fn terminal_provider_failures_preserve_the_closed_provider_neutral_class() -> AppResult<()> {
    for (generation, class) in [
        (20, TranslationFailureClass::Authentication),
        (21, TranslationFailureClass::PermissionDenied),
        (22, TranslationFailureClass::InvalidRequest),
        (23, TranslationFailureClass::UsageLimit),
    ] {
        let store = CaptionAggregateStore::default();
        let fixture = TranslationPolicyFixture::new();
        let (mut module, outcomes) = fixture
            .start_module(TranslationTarget::SimplifiedChinese)
            .map_err(fixture_error)?;
        fixture
            .admit(
                &module,
                reservation(&store, generation, "provider-failure", "private source")?,
                [AttemptScript::failure(class, false, None, false)],
            )
            .map_err(fixture_error)?;

        let outcome = outcomes
            .recv_timeout(TEST_TIMEOUT)
            .map_err(|_| AppError::state("Provider failure was not received."))?;
        let TranslationTerminalOutcome::Failed(failed) = outcome else {
            return Err(AppError::state(
                "Provider failure unexpectedly completed translation.",
            ));
        };
        assert_eq!(failed.class, class);
        stop_and_finish_fixture(&fixture, &mut module)?;
    }
    Ok(())
}

#[test]
fn failed_outcome_keeps_its_source_reserved_until_failure_is_recorded() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let fixture = TranslationPolicyFixture::new();
    let (mut module, outcomes) = fixture
        .start_module(TranslationTarget::SimplifiedChinese)
        .map_err(fixture_error)?;
    fixture
        .admit(
            &module,
            reservation(&store, 30, "failed", "source retained through failure")?,
            [AttemptScript::failure(
                TranslationFailureClass::ServiceUnavailable,
                false,
                None,
                false,
            )],
        )
        .map_err(fixture_error)?;

    let outcome = outcomes
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|_| AppError::state("Failure-retention outcome was not received."))?;
    let TranslationTerminalOutcome::Failed(failed) = outcome else {
        return Err(AppError::state(
            "Failure-retention work unexpectedly completed.",
        ));
    };
    for index in 0_u64..6 {
        accept_completed_source(
            &store,
            30,
            &format!("pressure-{index}"),
            format!("pressure source {index}"),
        )?;
    }
    assert!(
        store
            .snapshot()?
            .captions
            .iter()
            .any(|caption| caption.unit_id.as_deref() == Some("failed"))
    );

    let update = failed
        .fail()?
        .ok_or_else(|| AppError::state("Translation failure was not recorded."))?;
    assert!(matches!(
        update.change,
        crate::caption::CaptionAggregateChange::TranslationFailed(
            crate::caption::TranslationUnitSnapshot::Failed {
                reason_code: crate::caption::TranslationFailureReason::ProviderUnavailable,
                ..
            }
        )
    ));
    stop_and_finish_fixture(&fixture, &mut module)?;
    Ok(())
}

#[test]
fn provider_neutral_failure_classes_map_to_closed_aggregate_reasons() {
    let cases = [
        (
            TranslationFailureClass::Authentication,
            crate::caption::TranslationFailureReason::ProviderAuthenticationFailed,
        ),
        (
            TranslationFailureClass::PermissionDenied,
            crate::caption::TranslationFailureReason::ProviderPermissionDenied,
        ),
        (
            TranslationFailureClass::InvalidRequest,
            crate::caption::TranslationFailureReason::ProviderInvalidRequest,
        ),
        (
            TranslationFailureClass::RateLimited,
            crate::caption::TranslationFailureReason::ProviderRateLimited,
        ),
        (
            TranslationFailureClass::UsageLimit,
            crate::caption::TranslationFailureReason::ProviderUsageLimit,
        ),
        (
            TranslationFailureClass::ServiceUnavailable,
            crate::caption::TranslationFailureReason::ProviderUnavailable,
        ),
        (
            TranslationFailureClass::InvalidOutput,
            crate::caption::TranslationFailureReason::InvalidOutput,
        ),
        (
            TranslationFailureClass::DeadlineExceeded,
            crate::caption::TranslationFailureReason::DeadlineExceeded,
        ),
        (
            TranslationFailureClass::Unknown,
            crate::caption::TranslationFailureReason::Failed,
        ),
    ];

    for (class, expected_reason) in cases {
        assert_eq!(class.reason(), expected_reason);
    }
}

#[test]
fn retry_after_beyond_the_total_budget_finishes_without_an_early_retry() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let fixture = TranslationPolicyFixture::new();
    let (mut module, outcomes) = fixture
        .start_module_with_jitter(
            TranslationTarget::SimplifiedChinese,
            Duration::from_millis(300),
        )
        .map_err(fixture_error)?;
    fixture
        .admit(
            &module,
            reservation(&store, 6, "retry-after", "private source")?,
            [AttemptScript::failure(
                TranslationFailureClass::RateLimited,
                true,
                Some(Duration::from_secs(30)),
                false,
            )],
        )
        .map_err(fixture_error)?;

    assert!(matches!(
        outcomes.recv_timeout(TEST_TIMEOUT),
        Ok(TranslationTerminalOutcome::Failed(FailedTranslation {
            class: TranslationFailureClass::RateLimited,
            ..
        }))
    ));
    assert_eq!(stop_and_finish_fixture(&fixture, &mut module)?.len(), 1);
    assert!(fixture.delay_records().map_err(fixture_error)?.is_empty());
    Ok(())
}

#[test]
fn retry_after_within_the_total_budget_is_honored_without_jitter() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let fixture = TranslationPolicyFixture::new();
    let (mut module, outcomes) = fixture
        .start_module_with_jitter(
            TranslationTarget::SimplifiedChinese,
            Duration::from_millis(300),
        )
        .map_err(fixture_error)?;
    fixture
        .admit(
            &module,
            reservation(&store, 26, "retry-after-within-budget", "private source")?,
            [
                AttemptScript::failure(
                    TranslationFailureClass::RateLimited,
                    true,
                    Some(Duration::from_secs(2)),
                    false,
                ),
                AttemptScript::success("translated"),
            ],
        )
        .map_err(fixture_error)?;
    let delays = fixture
        .wait_for_delay_count(1, TEST_TIMEOUT)
        .map_err(fixture_error)?;
    assert_eq!(delays[0].requested(), Duration::from_secs(2));
    fixture
        .advance(Duration::from_secs(2))
        .map_err(fixture_error)?;

    assert!(matches!(
        outcomes.recv_timeout(TEST_TIMEOUT),
        Ok(TranslationTerminalOutcome::Completed(_))
    ));
    assert_eq!(stop_and_finish_fixture(&fixture, &mut module)?.len(), 2);
    Ok(())
}

#[test]
fn queued_work_does_not_shorten_retry_after_to_fit_the_remaining_budget() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let fixture = TranslationPolicyFixture::new();
    let (mut module, outcomes) = fixture
        .start_module(TranslationTarget::SimplifiedChinese)
        .map_err(fixture_error)?;
    fixture
        .admit(
            &module,
            reservation(&store, 12, "queue-one", "private source one")?,
            [AttemptScript::held_confirmed()],
        )
        .map_err(fixture_error)?;
    let first = fixture
        .wait_for_attempt_count(1, TEST_TIMEOUT)
        .map_err(fixture_error)?[0]
        .id();
    fixture
        .admit(
            &module,
            reservation(&store, 12, "queue-two", "private source two")?,
            [AttemptScript::held_confirmed()],
        )
        .map_err(fixture_error)?;
    fixture
        .admit(
            &module,
            reservation(&store, 12, "remaining-budget", "private source three")?,
            [AttemptScript::held_confirmed()],
        )
        .map_err(fixture_error)?;

    fixture
        .advance(Duration::from_millis(4_900))
        .map_err(fixture_error)?;
    fixture
        .complete(first, "translated-one")
        .map_err(fixture_error)?;
    assert!(matches!(
        outcomes.recv_timeout(TEST_TIMEOUT),
        Ok(TranslationTerminalOutcome::Completed(_))
    ));
    let second = fixture
        .wait_for_attempt_count(2, TEST_TIMEOUT)
        .map_err(fixture_error)?[1]
        .id();
    fixture
        .advance(Duration::from_millis(4_900))
        .map_err(fixture_error)?;
    fixture
        .complete(second, "translated-two")
        .map_err(fixture_error)?;
    assert!(matches!(
        outcomes.recv_timeout(TEST_TIMEOUT),
        Ok(TranslationTerminalOutcome::Completed(_))
    ));
    let third = fixture
        .wait_for_attempt_count(3, TEST_TIMEOUT)
        .map_err(fixture_error)?[2]
        .id();
    fixture
        .advance(Duration::from_millis(1_900))
        .map_err(fixture_error)?;
    fixture
        .fail(
            third,
            TranslationFailureClass::RateLimited,
            true,
            Some(Duration::from_secs(30)),
            false,
        )
        .map_err(fixture_error)?;

    let outcome = outcomes
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|_| AppError::state("Remaining-budget failure was not received."))?;
    assert!(matches!(
        outcome,
        TranslationTerminalOutcome::Failed(FailedTranslation {
            class: TranslationFailureClass::RateLimited,
            ..
        })
    ));
    fixture
        .wait_for_quiescence(third, TEST_TIMEOUT)
        .map_err(fixture_error)?;
    let records = stop_and_finish_fixture(&fixture, &mut module)?;
    assert_eq!(records.len(), 3);
    assert_eq!(
        (records[0].attempt_budget(), records[0].total_budget()),
        (Duration::from_secs(5), Duration::from_secs(12))
    );
    assert_eq!(records[1].started_at(), Duration::from_millis(4_900));
    assert_eq!(
        (records[1].attempt_budget(), records[1].total_budget()),
        (Duration::from_secs(5), Duration::from_millis(7_100))
    );
    assert_eq!(records[2].started_at(), Duration::from_millis(9_800));
    assert_eq!(
        (records[2].attempt_budget(), records[2].total_budget()),
        (Duration::from_millis(2_200), Duration::from_millis(2_200))
    );
    assert!(fixture.delay_records().map_err(fixture_error)?.is_empty());
    Ok(())
}

#[test]
fn unconfirmed_attempt_timeout_fails_closed_without_starting_more_work() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let fixture = TranslationPolicyFixture::new();
    let (mut module, outcomes) = fixture
        .start_module(TranslationTarget::SimplifiedChinese)
        .map_err(fixture_error)?;
    fixture
        .admit(
            &module,
            reservation(&store, 10, "attempt-timeout", "private source")?,
            [AttemptScript::held_unconfirmed()],
        )
        .map_err(fixture_error)?;
    let first = fixture
        .wait_for_attempt_count(1, TEST_TIMEOUT)
        .map_err(fixture_error)?[0]
        .id();
    fixture
        .admit(
            &module,
            reservation(&store, 10, "queued", "queued private source")?,
            [],
        )
        .map_err(fixture_error)?;
    fixture.advance(TOTAL_DEADLINE).map_err(fixture_error)?;

    for _ in 0..2 {
        let outcome = outcomes
            .recv_timeout(TEST_TIMEOUT)
            .map_err(|_| AppError::state("Attempt-timeout failure was not received."))?;
        assert!(matches!(
            outcome,
            TranslationTerminalOutcome::Failed(FailedTranslation {
                class: TranslationFailureClass::DeadlineExceeded,
                ..
            })
        ));
    }
    let cancelled = fixture
        .wait_for_cancellation(first, CancellationStatus::Unconfirmed, TEST_TIMEOUT)
        .map_err(fixture_error)?;
    assert_eq!(
        cancelled.terminal(),
        Some(fixture::AttemptTerminal::CancelledUnconfirmed)
    );
    assert_eq!(
        cancelled.cancellation(),
        Some(CancellationStatus::Unconfirmed)
    );
    assert!(!cancelled.is_quiesced());
    fixture.quiesce_unconfirmed(first).map_err(fixture_error)?;
    fixture
        .wait_for_quiescence(first, TEST_TIMEOUT)
        .map_err(fixture_error)?;
    assert_eq!(
        module
            .try_submit(reservation(&store, 10, "after-close", "private source")?)
            .map_err(|rejection| rejection.kind()),
        Err(TranslationSubmitError::Closed)
    );
    let owner = module.stop_and_confirm_owner_quiesced()?;
    assert_eq!(fixture.finish(owner).map_err(fixture_error)?.len(), 1);
    Ok(())
}

#[test]
fn late_ambiguous_result_keeps_its_fail_closed_semantics() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let fixture = TranslationPolicyFixture::new();
    let (mut module, outcomes) = fixture
        .start_module(TranslationTarget::SimplifiedChinese)
        .map_err(fixture_error)?;
    fixture
        .admit(
            &module,
            reservation(&store, 29, "late-ambiguous", "private source")?,
            [AttemptScript::non_cooperative_failure(
                TranslationFailureClass::DeadlineExceeded,
                false,
                None,
                true,
            )],
        )
        .map_err(fixture_error)?;
    let first = fixture
        .wait_for_attempt_count(1, TEST_TIMEOUT)
        .map_err(fixture_error)?[0]
        .id();
    fixture
        .admit(
            &module,
            reservation(&store, 29, "must-not-start", "queued private source")?,
            [],
        )
        .map_err(fixture_error)?;
    fixture
        .advance(ATTEMPT_DEADLINE + Duration::from_millis(1))
        .map_err(fixture_error)?;
    fixture
        .release_non_cooperative(first)
        .map_err(fixture_error)?;

    for _ in 0..2 {
        let outcome = outcomes
            .recv_timeout(TEST_TIMEOUT)
            .map_err(|_| AppError::state("Ambiguous failure was not received."))?;
        assert!(matches!(
            outcome,
            TranslationTerminalOutcome::Failed(FailedTranslation {
                class: TranslationFailureClass::DeadlineExceeded,
                ..
            })
        ));
    }
    fixture
        .wait_for_quiescence(first, TEST_TIMEOUT)
        .map_err(fixture_error)?;
    assert!(matches!(
        module
            .try_submit(reservation(
                &store,
                29,
                "closed-admission",
                "later private source",
            )?)
            .map_err(|rejection| rejection.kind()),
        Err(TranslationSubmitError::Closed)
    ));
    let owner = module.stop_and_confirm_owner_quiesced()?;
    assert_eq!(fixture.finish(owner).map_err(fixture_error)?.len(), 1);
    Ok(())
}

#[test]
fn total_deadline_starts_at_admission_and_includes_fifo_queue_time() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let fixture = TranslationPolicyFixture::new();
    let (mut module, outcomes) = fixture
        .start_module(TranslationTarget::SimplifiedChinese)
        .map_err(fixture_error)?;
    fixture
        .admit(
            &module,
            reservation(&store, 11, "active", "first private source")?,
            [AttemptScript::held_confirmed()],
        )
        .map_err(fixture_error)?;
    let first = fixture
        .wait_for_attempt_count(1, TEST_TIMEOUT)
        .map_err(fixture_error)?[0]
        .id();
    fixture
        .admit(
            &module,
            reservation(&store, 11, "queued", "second private source")?,
            [],
        )
        .map_err(fixture_error)?;
    fixture.advance(TOTAL_DEADLINE).map_err(fixture_error)?;

    for _ in 0..2 {
        let outcome = outcomes
            .recv_timeout(TEST_TIMEOUT)
            .map_err(|_| AppError::state("Deadline failure was not received."))?;
        assert!(matches!(
            outcome,
            TranslationTerminalOutcome::Failed(FailedTranslation {
                class: TranslationFailureClass::DeadlineExceeded,
                ..
            })
        ));
    }
    fixture
        .wait_for_quiescence(first, TEST_TIMEOUT)
        .map_err(fixture_error)?;
    let owner = module.stop_and_confirm_owner_quiesced()?;
    assert_eq!(fixture.finish(owner).map_err(fixture_error)?.len(), 1);
    Ok(())
}

#[test]
fn empty_and_oversized_outputs_are_safe_terminal_failures() -> AppResult<()> {
    for (generation, output) in [
        (7, "   ".to_string()),
        (8, "x".repeat(TRANSLATION_BYTE_LIMIT + 1)),
    ] {
        let store = CaptionAggregateStore::default();
        let fixture = TranslationPolicyFixture::new();
        let (mut module, outcomes) = fixture
            .start_module(TranslationTarget::SimplifiedChinese)
            .map_err(fixture_error)?;
        fixture
            .admit(
                &module,
                reservation(&store, generation, "invalid", "private source")?,
                [AttemptScript::success(output)],
            )
            .map_err(fixture_error)?;

        let outcome = outcomes
            .recv_timeout(TEST_TIMEOUT)
            .map_err(|_| AppError::state("Invalid-output failure was not received."))?;
        assert!(matches!(
            outcome,
            TranslationTerminalOutcome::Failed(FailedTranslation {
                class: TranslationFailureClass::InvalidOutput,
                ..
            })
        ));
        stop_and_finish_fixture(&fixture, &mut module)?;
    }
    Ok(())
}

#[test]
fn terminal_debug_does_not_expose_source_translation_or_provider_body() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let fixture = TranslationPolicyFixture::new();
    let (mut module, outcomes) = fixture
        .start_module(TranslationTarget::SimplifiedChinese)
        .map_err(fixture_error)?;
    fixture
        .admit(
            &module,
            reservation(&store, 9, "debug", "private source")?,
            [AttemptScript::success("private translation")],
        )
        .map_err(fixture_error)?;

    let outcome = outcomes
        .recv_timeout(TEST_TIMEOUT)
        .map_err(|_| AppError::state("Debug test outcome was not received."))?;
    let debug = format!("{outcome:?}");
    assert!(!debug.contains("private source"));
    assert!(!debug.contains("private translation"));
    let fixture_debug = format!("{:?}", stop_and_finish_fixture(&fixture, &mut module)?);
    assert!(!fixture_debug.contains("private source"));
    assert!(!fixture_debug.contains("private translation"));
    Ok(())
}

#[test]
fn production_retry_jitter_stays_within_twenty_percent() {
    let jitter = BoundedRetryJitter::new();
    for _ in 0..128 {
        let delay = jitter.delay(RETRY_BASE_DELAY);
        assert!(delay >= Duration::from_millis(200));
        assert!(delay <= Duration::from_millis(300));
    }
}

#[test]
fn dropping_outcome_receiver_stops_admission_and_releases_work() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let fixture = TranslationPolicyFixture::new();
    let (mut module, outcomes) = fixture
        .start_module(TranslationTarget::SimplifiedChinese)
        .map_err(fixture_error)?;
    fixture
        .admit(
            &module,
            reservation(&store, 15, "receiver-drop", "private source")?,
            [AttemptScript::held_confirmed()],
        )
        .map_err(fixture_error)?;
    let first = fixture
        .wait_for_attempt_count(1, TEST_TIMEOUT)
        .map_err(fixture_error)?[0]
        .id();

    drop(outcomes);
    fixture
        .wait_for_quiescence(first, TEST_TIMEOUT)
        .map_err(fixture_error)?;
    assert_eq!(
        module
            .try_submit(reservation(&store, 15, "late", "late source")?)
            .map_err(|rejection| rejection.kind()),
        Err(TranslationSubmitError::Stopped)
    );
    let owner = module.stop_and_confirm_owner_quiesced()?;
    fixture.finish(owner).map_err(fixture_error)?;
    Ok(())
}

#[test]
fn stop_wins_an_admission_race_after_submission_preparation() -> AppResult<()> {
    let store = CaptionAggregateStore::default();
    let fixture = TranslationPolicyFixture::new();
    let (mut module, _outcomes) = fixture
        .start_module(TranslationTarget::SimplifiedChinese)
        .map_err(fixture_error)?;
    let prepared = reservation(&store, 16, "race", "private source")?;
    let result = module.try_submit_with_stop_hook(prepared);

    assert_eq!(
        result.map_err(|rejection| rejection.kind()),
        Err(TranslationSubmitError::Stopped)
    );
    stop_and_finish_fixture(&fixture, &mut module)?;
    Ok(())
}
