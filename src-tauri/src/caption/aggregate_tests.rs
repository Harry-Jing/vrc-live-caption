use super::super::contract::SourceSnapshotRef;
use super::*;

fn source_caption(
    generation: u64,
    stream_id: &str,
    unit_id: &str,
    text: impl Into<String>,
    state: CaptionState,
    started_at_ms: u64,
) -> CaptionSnapshot {
    CaptionSnapshot {
        generation,
        stream_id: stream_id.to_string(),
        unit_id: Some(unit_id.to_string()),
        lane: CaptionLane::Source,
        revision: 1,
        text: text.into(),
        state,
        language: Some("en".to_string()),
        source_ref: None,
        unit_started_at_ms: Some(started_at_ms),
        timestamp_ms: started_at_ms.saturating_add(1),
    }
}

fn translation_caption(
    source: &CaptionSnapshot,
    text: impl Into<String>,
    state: CaptionState,
) -> crate::error::AppResult<CaptionSnapshot> {
    let unit_id = source
        .unit_id
        .clone()
        .ok_or_else(|| crate::error::AppError::state("Test source caption had no unit id."))?;
    Ok(CaptionSnapshot {
        lane: CaptionLane::Translation,
        revision: 1,
        text: text.into(),
        state,
        language: Some("zh".to_string()),
        source_ref: Some(SourceSnapshotRef {
            generation: source.generation,
            stream_id: source.stream_id.clone(),
            unit_id,
            revision: source.revision,
        }),
        timestamp_ms: source.timestamp_ms.saturating_add(1),
        ..source.clone()
    })
}

#[test]
fn completed_source_lane_does_not_close_the_correlated_translation_lane()
-> crate::error::AppResult<()> {
    let store = CaptionAggregateStore::default();
    let active = store
        .begin_generation(1)?
        .active_stream
        .ok_or_else(|| crate::error::AppError::state("Generation 1 was not active."))?;
    store.start_unit(1, &active.stream_id, "speech-1".to_string(), 10)?;

    let source = source_caption(
        1,
        &active.stream_id,
        "speech-1",
        "source",
        CaptionState::Completed,
        10,
    );
    let (source_update, reservation) = store
        .accept_completed_source_for_translation(source.clone())?
        .ok_or_else(|| crate::error::AppError::state("Source lane was rejected."))?;
    assert!(source_update.snapshot.open_source_units.is_empty());

    let update = reservation
        .complete_translation(
            "translation".to_string(),
            "zh".to_string(),
            source.timestamp_ms.saturating_add(1),
        )?
        .ok_or_else(|| crate::error::AppError::state("Translation lane was rejected."))?;
    let snapshot = update.snapshot;

    assert_eq!(snapshot.captions.len(), 2);
    assert_eq!(
        snapshot
            .captions
            .iter()
            .map(|caption| caption.lane)
            .collect::<Vec<_>>(),
        [CaptionLane::Source, CaptionLane::Translation]
    );
    assert_eq!(snapshot.captions[1].text, "translation");

    Ok(())
}

#[test]
fn completed_source_reservation_rejects_non_terminal_or_non_source_captions()
-> crate::error::AppResult<()> {
    let store = CaptionAggregateStore::default();
    let active = store
        .begin_generation(1)?
        .active_stream
        .ok_or_else(|| crate::error::AppError::state("Generation 1 was not active."))?;
    store.start_unit(1, &active.stream_id, "ongoing".to_string(), 10)?;

    let ongoing = source_caption(
        1,
        &active.stream_id,
        "ongoing",
        "source",
        CaptionState::Ongoing,
        10,
    );
    assert!(
        store
            .accept_completed_source_for_translation(ongoing.clone())?
            .is_none()
    );
    assert!(store.accept_caption(ongoing.clone())?.is_some());

    let mut translation = ongoing;
    translation.lane = CaptionLane::Translation;
    translation.state = CaptionState::Completed;
    translation.source_ref = Some(SourceSnapshotRef {
        generation: 1,
        stream_id: active.stream_id,
        unit_id: "ongoing".to_string(),
        revision: 1,
    });
    assert!(
        store
            .accept_completed_source_for_translation(translation)?
            .is_none()
    );

    Ok(())
}

#[test]
fn lane_linkage_shape_is_enforced_at_aggregate_admission() -> crate::error::AppResult<()> {
    let store = CaptionAggregateStore::default();
    let active = store
        .begin_generation(1)?
        .active_stream
        .ok_or_else(|| crate::error::AppError::state("Generation 1 was not active."))?;
    store.start_unit(1, &active.stream_id, "source".to_string(), 10)?;

    let source = source_caption(
        1,
        &active.stream_id,
        "source",
        "source",
        CaptionState::Completed,
        10,
    );
    assert!(store.accept_caption(source.clone())?.is_some());

    let mut unlinked_translation =
        translation_caption(&source, "translation", CaptionState::Completed)?;
    unlinked_translation.source_ref = None;
    assert!(store.accept_caption(unlinked_translation)?.is_none());

    store.start_unit(1, &active.stream_id, "next-source".to_string(), 20)?;
    let mut linked_source = source_caption(
        1,
        &active.stream_id,
        "next-source",
        "next source",
        CaptionState::Ongoing,
        20,
    );
    linked_source.source_ref = Some(SourceSnapshotRef {
        generation: source.generation,
        stream_id: source.stream_id,
        unit_id: source.unit_id.unwrap_or_default(),
        revision: source.revision,
    });
    assert!(store.accept_caption(linked_source)?.is_none());

    Ok(())
}

#[test]
fn translation_outcomes_produce_the_shared_v2_aggregate_snapshot() -> crate::error::AppResult<()> {
    let store = CaptionAggregateStore::default();
    let started = store.begin_generation(7)?;
    let active = started
        .active_stream
        .ok_or_else(|| crate::error::AppError::state("Test generation did not become active."))?;

    assert_eq!(active.stream_id, "recognition-7-1");
    assert!(
        store
            .start_unit(7, &active.stream_id, "speech-7-1".to_string(), 1_000)?
            .is_some()
    );
    let completed_source = CaptionSnapshot {
        generation: 7,
        stream_id: active.stream_id.clone(),
        unit_id: Some("speech-7-1".to_string()),
        lane: CaptionLane::Source,
        revision: 1,
        text: "Source with a completed translation.".to_string(),
        state: CaptionState::Completed,
        language: Some("en".to_string()),
        source_ref: None,
        unit_started_at_ms: Some(1_000),
        timestamp_ms: 1_200,
    };
    let (_, reservation) = store
        .accept_completed_source_for_translation(completed_source.clone())?
        .ok_or_else(|| crate::error::AppError::state("Source was not reserved."))?;
    assert!(
        reservation
            .complete_translation(
                "已完成翻译的原文。".to_string(),
                "zh-Hans".to_string(),
                completed_source.timestamp_ms.saturating_add(1),
            )?
            .is_some()
    );

    assert!(
        store
            .start_unit(7, &active.stream_id, "speech-7-2".to_string(), 1_200)?
            .is_some()
    );
    let pending_source = CaptionSnapshot {
        unit_id: Some("speech-7-2".to_string()),
        text: "Source awaiting translation.".to_string(),
        unit_started_at_ms: Some(1_200),
        timestamp_ms: 1_400,
        ..completed_source.clone()
    };
    let (_, _pending_reservation) = store
        .accept_completed_source_for_translation(pending_source)?
        .ok_or_else(|| crate::error::AppError::state("Source was not reserved."))?;

    assert!(
        store
            .start_unit(7, &active.stream_id, "speech-7-3".to_string(), 1_400)?
            .is_some()
    );
    let failed_source = CaptionSnapshot {
        unit_id: Some("speech-7-3".to_string()),
        text: "Source whose translation failed.".to_string(),
        unit_started_at_ms: Some(1_400),
        timestamp_ms: 1_600,
        ..completed_source
    };
    let (_, failed_reservation) = store
        .accept_completed_source_for_translation(failed_source)?
        .ok_or_else(|| crate::error::AppError::state("Source was not reserved."))?;
    assert!(
        failed_reservation
            .fail_translation(TranslationFailureReason::ProviderUnavailable)?
            .is_some()
    );

    let expected = serde_json::from_str::<serde_json::Value>(include_str!(
        "../../../contracts/caption-aggregate-snapshot-v2.json"
    ))
    .map_err(|error| crate::error::AppError::state(error.to_string()))?;
    let actual = serde_json::to_value(store.snapshot()?)
        .map_err(|error| crate::error::AppError::state(error.to_string()))?;

    assert_eq!(actual, expected);

    Ok(())
}

#[test]
fn stop_and_new_generation_reject_old_writers_without_losing_completed_history()
-> crate::error::AppResult<()> {
    let store = CaptionAggregateStore::default();
    let active = store
        .begin_generation(1)?
        .active_stream
        .ok_or_else(|| crate::error::AppError::state("Generation 1 was not active."))?;
    store.start_unit(1, &active.stream_id, "completed".to_string(), 10)?;
    let completed = CaptionSnapshot {
        generation: 1,
        stream_id: active.stream_id.clone(),
        unit_id: Some("completed".to_string()),
        lane: CaptionLane::Source,
        revision: 1,
        text: "kept".to_string(),
        state: CaptionState::Completed,
        language: Some("en".to_string()),
        source_ref: None,
        unit_started_at_ms: Some(10),
        timestamp_ms: 20,
    };
    assert!(store.accept_caption(completed.clone())?.is_some());
    assert!(
        store
            .accept_caption(CaptionSnapshot {
                revision: 2,
                text: "must not reopen".to_string(),
                state: CaptionState::Ongoing,
                ..completed.clone()
            })?
            .is_none()
    );

    store.start_unit(1, &active.stream_id, "ongoing".to_string(), 30)?;
    assert!(
        store
            .accept_caption(CaptionSnapshot {
                unit_id: Some("ongoing".to_string()),
                revision: 2,
                text: "discarded on stop".to_string(),
                state: CaptionState::Ongoing,
                unit_started_at_ms: Some(30),
                timestamp_ms: 40,
                ..completed.clone()
            })?
            .is_some()
    );
    let stopped = store
        .close_generation(1)?
        .ok_or_else(|| crate::error::AppError::state("Generation 1 did not close."))?;

    assert!(stopped.active_stream.is_none());
    assert!(stopped.open_source_units.is_empty());
    assert_eq!(stopped.captions.len(), 1);
    assert_eq!(stopped.captions[0].text, "kept");
    assert!(store.accept_caption(completed.clone())?.is_none());

    let next = store.begin_generation(2)?;
    assert_eq!(
        next.active_stream
            .as_ref()
            .map(|active| active.stream_id.as_str()),
        Some("recognition-2-1")
    );
    assert!(
        store
            .start_unit(1, &active.stream_id, "late".to_string(), 50)?
            .is_none()
    );
    assert_eq!(store.begin_generation(1)?, next);

    let next_active = next
        .active_stream
        .ok_or_else(|| crate::error::AppError::state("Generation 2 was not active."))?;
    store.start_unit(2, &next_active.stream_id, "no-result".to_string(), 60)?;
    assert!(
        store
            .abort_source_unit(2, &next_active.stream_id, "no-result")?
            .is_some()
    );
    assert!(
        store
            .start_unit(2, &next_active.stream_id, "no-result".to_string(), 60,)?
            .is_none()
    );
    assert!(
        store
            .accept_caption(CaptionSnapshot {
                generation: 2,
                stream_id: next_active.stream_id,
                unit_id: Some("no-result".to_string()),
                revision: 1,
                text: "late".to_string(),
                timestamp_ms: 70,
                unit_started_at_ms: Some(60),
                ..completed
            })?
            .is_none()
    );

    Ok(())
}

#[test]
fn unitless_ongoing_caption_replaces_by_revision_but_cannot_complete() -> crate::error::AppResult<()>
{
    let store = CaptionAggregateStore::default();
    let active = store
        .begin_generation(3)?
        .active_stream
        .ok_or_else(|| crate::error::AppError::state("Generation 3 was not active."))?;
    let first = CaptionSnapshot {
        generation: 3,
        stream_id: active.stream_id,
        unit_id: None,
        lane: CaptionLane::Source,
        revision: 1,
        text: "first full snapshot".to_string(),
        state: CaptionState::Ongoing,
        language: Some("en".to_string()),
        source_ref: None,
        unit_started_at_ms: None,
        timestamp_ms: 10,
    };

    assert!(store.accept_caption(first.clone())?.is_some());
    assert!(store.accept_caption(first.clone())?.is_none());
    assert!(
        store
            .accept_caption(CaptionSnapshot {
                revision: 0,
                text: "older".to_string(),
                ..first.clone()
            })?
            .is_none()
    );
    assert!(
        store
            .accept_caption(CaptionSnapshot {
                revision: 2,
                text: "replacement full snapshot".to_string(),
                timestamp_ms: 20,
                ..first.clone()
            })?
            .is_some()
    );
    assert!(
        store
            .accept_caption(CaptionSnapshot {
                revision: 1,
                text: "stale full snapshot".to_string(),
                timestamp_ms: 25,
                ..first.clone()
            })?
            .is_none()
    );
    assert!(
        store
            .accept_caption(CaptionSnapshot {
                revision: 3,
                text: "unitless completion is invalid".to_string(),
                state: CaptionState::Completed,
                timestamp_ms: 30,
                ..first
            })?
            .is_none()
    );

    let snapshot = store.snapshot()?;
    assert!(snapshot.open_source_units.is_empty());
    assert_eq!(snapshot.captions.len(), 1);
    assert_eq!(snapshot.captions[0].revision, 2);
    assert_eq!(snapshot.captions[0].text, "replacement full snapshot");
    assert_eq!(snapshot.captions[0].state, CaptionState::Ongoing);

    Ok(())
}

#[test]
fn completed_history_keeps_the_five_newest_units_in_newest_first_order()
-> crate::error::AppResult<()> {
    let store = CaptionAggregateStore::default();
    let active = store
        .begin_generation(4)?
        .active_stream
        .ok_or_else(|| crate::error::AppError::state("Generation 4 was not active."))?;

    for index in 0_u64..7 {
        let unit_id = format!("unit-{index}");
        store.start_unit(4, &active.stream_id, unit_id.clone(), index * 10)?;
        assert!(
            store
                .accept_caption(CaptionSnapshot {
                    generation: 4,
                    stream_id: active.stream_id.clone(),
                    unit_id: Some(unit_id),
                    lane: CaptionLane::Source,
                    revision: 1,
                    text: format!("caption {index}"),
                    state: CaptionState::Completed,
                    language: Some("en".to_string()),
                    source_ref: None,
                    unit_started_at_ms: Some(index * 10),
                    timestamp_ms: index * 10 + 1,
                })?
                .is_some()
        );
    }

    let snapshot = store.snapshot()?;
    assert_eq!(snapshot.captions.len(), 5);
    assert_eq!(
        snapshot
            .captions
            .iter()
            .map(|caption| caption.unit_id.as_deref().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["unit-6", "unit-5", "unit-4", "unit-3", "unit-2"]
    );

    Ok(())
}

#[test]
fn completed_history_eviction_uses_unit_order_when_completion_arrives_out_of_order()
-> crate::error::AppResult<()> {
    let store = CaptionAggregateStore::default();
    let active = store
        .begin_generation(5)?
        .active_stream
        .ok_or_else(|| crate::error::AppError::state("Generation 5 was not active."))?;

    for index in 0_u64..6 {
        assert!(
            store
                .start_unit(5, &active.stream_id, format!("unit-{index}"), index * 10,)?
                .is_some()
        );
    }

    for index in (0_u64..6).rev() {
        assert!(
            store
                .accept_caption(source_caption(
                    5,
                    &active.stream_id,
                    &format!("unit-{index}"),
                    format!("caption {index}"),
                    CaptionState::Completed,
                    index * 10,
                ))?
                .is_some()
        );
    }

    assert_eq!(
        store
            .snapshot()?
            .captions
            .iter()
            .map(|caption| caption.unit_id.as_deref().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["unit-5", "unit-4", "unit-3", "unit-2", "unit-1"]
    );

    Ok(())
}

#[test]
fn completing_an_older_unit_preserves_application_unit_order_after_clock_rollback()
-> crate::error::AppResult<()> {
    let store = CaptionAggregateStore::default();
    let active = store
        .begin_generation(6)?
        .active_stream
        .ok_or_else(|| crate::error::AppError::state("Generation 6 was not active."))?;

    assert!(
        store
            .start_unit(6, &active.stream_id, "older".to_string(), 200)?
            .is_some()
    );
    assert!(
        store
            .start_unit(6, &active.stream_id, "newer".to_string(), 100)?
            .is_some()
    );

    let mut older = source_caption(
        6,
        &active.stream_id,
        "older",
        "older draft",
        CaptionState::Ongoing,
        200,
    );
    older.timestamp_ms = 250;
    let mut newer = source_caption(
        6,
        &active.stream_id,
        "newer",
        "newer draft",
        CaptionState::Ongoing,
        100,
    );
    newer.timestamp_ms = 150;
    assert!(store.accept_caption(older.clone())?.is_some());
    assert!(store.accept_caption(newer)?.is_some());
    assert!(
        store
            .accept_caption(CaptionSnapshot {
                revision: 2,
                text: "older completed".to_string(),
                state: CaptionState::Completed,
                timestamp_ms: 300,
                ..older
            })?
            .is_some()
    );

    assert_eq!(
        store
            .snapshot()?
            .captions
            .iter()
            .map(|caption| caption.unit_id.as_deref().unwrap_or_default())
            .collect::<Vec<_>>(),
        ["newer", "older"]
    );

    Ok(())
}

#[test]
fn unit_order_is_generation_scoped_when_unit_ids_are_reused() -> crate::error::AppResult<()> {
    let store = CaptionAggregateStore::default();
    let first = store
        .begin_generation(7)?
        .active_stream
        .ok_or_else(|| crate::error::AppError::state("Generation 7 was not active."))?;

    for (unit_id, started_at_ms) in [("first", 10), ("second", 20)] {
        assert!(
            store
                .start_unit(7, &first.stream_id, unit_id.to_string(), started_at_ms)?
                .is_some()
        );
        assert!(
            store
                .accept_caption(source_caption(
                    7,
                    &first.stream_id,
                    unit_id,
                    format!("generation 7 {unit_id}"),
                    CaptionState::Completed,
                    started_at_ms,
                ))?
                .is_some()
        );
    }
    assert!(store.close_generation(7)?.is_some());

    let second = store
        .begin_generation(8)?
        .active_stream
        .ok_or_else(|| crate::error::AppError::state("Generation 8 was not active."))?;
    assert!(
        store
            .start_unit(8, &second.stream_id, "second".to_string(), 200)?
            .is_some()
    );
    assert!(
        store
            .start_unit(8, &second.stream_id, "first".to_string(), 100)?
            .is_some()
    );

    let mut older = source_caption(
        8,
        &second.stream_id,
        "second",
        "generation 8 second",
        CaptionState::Ongoing,
        200,
    );
    older.timestamp_ms = 250;
    assert!(store.accept_caption(older.clone())?.is_some());
    let mut newer = source_caption(
        8,
        &second.stream_id,
        "first",
        "generation 8 first",
        CaptionState::Ongoing,
        100,
    );
    newer.timestamp_ms = 150;
    assert!(store.accept_caption(newer)?.is_some());
    assert!(
        store
            .accept_caption(CaptionSnapshot {
                revision: 2,
                text: "generation 8 second revised".to_string(),
                timestamp_ms: 300,
                ..older
            })?
            .is_some()
    );

    assert_eq!(
        store
            .snapshot()?
            .captions
            .iter()
            .map(|caption| (
                caption.generation,
                caption.unit_id.as_deref().unwrap_or_default()
            ))
            .collect::<Vec<_>>(),
        [(8, "first"), (8, "second"), (7, "second"), (7, "first")]
    );

    Ok(())
}

#[test]
fn terminal_lane_replay_guard_stays_bounded_during_a_long_generation() -> crate::error::AppResult<()>
{
    let store = CaptionAggregateStore::default();
    let active = store
        .begin_generation(5)?
        .active_stream
        .ok_or_else(|| crate::error::AppError::state("Generation 5 was not active."))?;
    let expected_replay_bound = TERMINAL_LANE_REPLAY_LIMIT;
    let retained_unit_id = "retained-completed".to_string();
    store.start_unit(5, &active.stream_id, retained_unit_id.clone(), 0)?;
    assert!(
        store
            .accept_caption(CaptionSnapshot {
                generation: 5,
                stream_id: active.stream_id.clone(),
                unit_id: Some(retained_unit_id.clone()),
                lane: CaptionLane::Source,
                revision: 1,
                text: "retained".to_string(),
                state: CaptionState::Completed,
                language: Some("en".to_string()),
                source_ref: None,
                unit_started_at_ms: Some(0),
                timestamp_ms: 1,
            })?
            .is_some()
    );

    for index in 0..expected_replay_bound + 3 {
        let unit_id = format!("no-result-{index}");
        assert!(
            store
                .start_unit(5, &active.stream_id, unit_id.clone(), index as u64)?
                .is_some()
        );
        assert!(
            store
                .abort_source_unit(5, &active.stream_id, &unit_id)?
                .is_some()
        );
    }

    assert!(
        store
            .start_unit(5, &active.stream_id, retained_unit_id, 1_000)?
            .is_none()
    );
    assert!(
        store
            .start_unit(
                5,
                &active.stream_id,
                format!("no-result-{}", expected_replay_bound + 2),
                1_001,
            )?
            .is_none()
    );

    let state = store.lock()?;
    assert_eq!(state.recent_terminal_lanes.len(), expected_replay_bound);

    Ok(())
}

#[test]
fn accepted_completion_change_survives_immediate_snapshot_history_trimming()
-> crate::error::AppResult<()> {
    let store = CaptionAggregateStore::default();
    let active = store
        .begin_generation(9)?
        .active_stream
        .ok_or_else(|| crate::error::AppError::state("Generation 9 was not active."))?;

    for index in 0_u64..6 {
        store.start_unit(9, &active.stream_id, format!("unit-{index}"), index * 10)?;
    }
    for index in (1_u64..6).rev() {
        store.accept_caption(source_caption(
            9,
            &active.stream_id,
            &format!("unit-{index}"),
            format!("caption {index}"),
            CaptionState::Completed,
            index * 10,
        ))?;
    }

    let update = store
        .accept_caption(source_caption(
            9,
            &active.stream_id,
            "unit-0",
            "must still publish",
            CaptionState::Completed,
            0,
        ))?
        .ok_or_else(|| crate::error::AppError::state("Old completion was not accepted."))?;

    assert!(
        update
            .snapshot
            .captions
            .iter()
            .all(|caption| caption.unit_id.as_deref() != Some("unit-0"))
    );
    assert!(matches!(
        update.change,
        CaptionAggregateChange::CaptionAccepted(CaptionSnapshot {
            unit_id: Some(ref unit_id),
            ref text,
            state: CaptionState::Completed,
            ..
        }) if unit_id == "unit-0" && text == "must still publish"
    ));

    Ok(())
}

#[test]
fn unit_admissions_return_the_exact_open_and_abort_changes() -> crate::error::AppResult<()> {
    let store = CaptionAggregateStore::default();
    let active = store
        .begin_generation(10)?
        .active_stream
        .ok_or_else(|| crate::error::AppError::state("Generation 10 was not active."))?;

    let opened = store
        .start_unit(10, &active.stream_id, "speech-10".to_string(), 123)?
        .ok_or_else(|| crate::error::AppError::state("Caption unit was not opened."))?;
    assert!(matches!(
        opened.change,
        CaptionAggregateChange::SourceUnitOpened(OpenSourceUnit {
            ref unit_id,
            started_at_ms: 123,
        }) if unit_id == "speech-10"
    ));
    assert_eq!(opened.snapshot.open_source_units.len(), 1);

    let aborted = store
        .abort_source_unit(10, &active.stream_id, "speech-10")?
        .ok_or_else(|| crate::error::AppError::state("Source unit was not aborted."))?;
    assert!(matches!(
        aborted.change,
        CaptionAggregateChange::SourceUnitAborted { ref unit_id } if unit_id == "speech-10"
    ));
    assert!(aborted.snapshot.open_source_units.is_empty());

    Ok(())
}

#[test]
fn completed_source_reservation_is_atomic_with_admission_and_pins_history()
-> crate::error::AppResult<()> {
    let store = CaptionAggregateStore::default();
    let active = store
        .begin_generation(11)?
        .active_stream
        .ok_or_else(|| crate::error::AppError::state("Generation 11 was not active."))?;

    for index in 0_u64..7 {
        assert!(
            store
                .start_unit(11, &active.stream_id, format!("unit-{index}"), index * 10,)?
                .is_some()
        );
    }

    let (_, first_reservation) = store
        .accept_completed_source_for_translation(source_caption(
            11,
            &active.stream_id,
            "unit-0",
            "pinned source text",
            CaptionState::Completed,
            0,
        ))?
        .ok_or_else(|| crate::error::AppError::state("Source was not reserved."))?;
    let (_, second_reservation) = store
        .accept_completed_source_for_translation(source_caption(
            11,
            &active.stream_id,
            "unit-1",
            "second pinned source text",
            CaptionState::Completed,
            10,
        ))?
        .ok_or_else(|| crate::error::AppError::state("Second source was not reserved."))?;
    {
        let state = store.lock()?;
        assert_eq!(
            state
                .pinned_sources
                .iter()
                .map(|source| (
                    source.unit.generation,
                    source.unit.stream_id.as_str(),
                    source.unit.unit_id.as_str(),
                    source.revision,
                ))
                .collect::<Vec<_>>(),
            [
                (11, active.stream_id.as_str(), "unit-0", 1),
                (11, active.stream_id.as_str(), "unit-1", 1),
            ]
        );
    }
    for index in 2_u64..7 {
        assert!(
            store
                .accept_caption(source_caption(
                    11,
                    &active.stream_id,
                    &format!("unit-{index}"),
                    format!("caption {index}"),
                    CaptionState::Completed,
                    index * 10,
                ))?
                .is_some()
        );
    }

    let pinned = store.snapshot()?;
    assert_eq!(pinned.captions.len(), COMPLETED_UNIT_LIMIT + 2);
    for unit_id in ["unit-0", "unit-1"] {
        assert!(
            pinned
                .captions
                .iter()
                .any(|caption| caption.unit_id.as_deref() == Some(unit_id))
        );
    }

    drop(first_reservation);

    let one_released = store.snapshot()?;
    assert_eq!(one_released.captions.len(), COMPLETED_UNIT_LIMIT + 1);
    assert!(
        one_released
            .captions
            .iter()
            .any(|caption| caption.unit_id.as_deref() == Some("unit-0"))
    );
    assert!(
        one_released
            .captions
            .iter()
            .any(|caption| caption.unit_id.as_deref() == Some("unit-1"))
    );
    assert!(one_released.translation_units.iter().any(|unit| matches!(
        unit,
        TranslationUnitSnapshot::Failed {
            reason_code: TranslationFailureReason::Stopped,
            source_ref,
        } if source_ref.unit_id == "unit-0"
    )));

    drop(second_reservation);

    let all_released = store.snapshot()?;
    assert_eq!(all_released.captions.len(), COMPLETED_UNIT_LIMIT);
    assert!(
        all_released
            .captions
            .iter()
            .any(|caption| caption.unit_id.as_deref() == Some("unit-1"))
    );
    assert!(all_released.translation_units.iter().any(|unit| matches!(
        unit,
        TranslationUnitSnapshot::Failed {
            reason_code: TranslationFailureReason::Stopped,
            source_ref,
        } if source_ref.unit_id == "unit-1"
    )));

    Ok(())
}

#[test]
fn failed_translation_is_recorded_before_its_exact_source_can_be_trimmed()
-> crate::error::AppResult<()> {
    let store = CaptionAggregateStore::default();
    let active = store
        .begin_generation(12)?
        .active_stream
        .ok_or_else(|| crate::error::AppError::state("Generation 12 was not active."))?;
    for index in 0_u64..6 {
        store.start_unit(12, &active.stream_id, format!("unit-{index}"), index * 10)?;
    }
    let source = source_caption(
        12,
        &active.stream_id,
        "unit-0",
        "private source",
        CaptionState::Completed,
        0,
    );
    let (pending, reservation) = store
        .accept_completed_source_for_translation(source.clone())?
        .ok_or_else(|| crate::error::AppError::state("Source was not reserved."))?;
    assert_eq!(
        pending.snapshot.translation_units,
        [TranslationUnitSnapshot::Pending {
            source_ref: SourceSnapshotRef {
                generation: 12,
                stream_id: active.stream_id.clone(),
                unit_id: "unit-0".to_string(),
                revision: 1,
            },
        }]
    );

    for index in 1_u64..6 {
        store.accept_caption(source_caption(
            12,
            &active.stream_id,
            &format!("unit-{index}"),
            format!("caption {index}"),
            CaptionState::Completed,
            index * 10,
        ))?;
    }

    let failed = reservation
        .fail_translation(TranslationFailureReason::ProviderUnavailable)?
        .ok_or_else(|| crate::error::AppError::state("Translation failure was not recorded."))?;
    assert!(failed.snapshot.captions.contains(&source));
    assert_eq!(failed.snapshot.captions.len(), COMPLETED_UNIT_LIMIT);
    assert_eq!(
        failed.snapshot.translation_units,
        [TranslationUnitSnapshot::Failed {
            source_ref: SourceSnapshotRef {
                generation: 12,
                stream_id: active.stream_id,
                unit_id: "unit-0".to_string(),
                revision: 1,
            },
            reason_code: TranslationFailureReason::ProviderUnavailable,
        }]
    );

    Ok(())
}

#[test]
fn reservation_constructs_the_exact_completed_translation_without_exposing_text_in_debug()
-> crate::error::AppResult<()> {
    let store = CaptionAggregateStore::default();
    let active = store
        .begin_generation(12)?
        .active_stream
        .ok_or_else(|| crate::error::AppError::state("Generation 12 was not active."))?;
    store.start_unit(12, &active.stream_id, "speech-12".to_string(), 100)?;

    let mut ongoing = source_caption(
        12,
        &active.stream_id,
        "speech-12",
        "draft",
        CaptionState::Ongoing,
        100,
    );
    ongoing.revision = 2;
    assert!(store.accept_caption(ongoing.clone())?.is_some());
    let source = CaptionSnapshot {
        revision: 3,
        text: "private completed source".to_string(),
        state: CaptionState::Completed,
        timestamp_ms: 130,
        ..ongoing
    };
    let (_, reservation) = store
        .accept_completed_source_for_translation(source.clone())?
        .ok_or_else(|| crate::error::AppError::state("Source was not reserved."))?;

    assert_eq!(reservation.source(), &source);
    let debug = format!("{reservation:?}");
    assert!(!debug.contains("private completed source"));
    assert!(debug.contains("speech-12"));

    let update = reservation
        .complete_translation(
            "private translation".to_string(),
            "zh-Hans".to_string(),
            150,
        )?
        .ok_or_else(|| crate::error::AppError::state("Translation was not finalized."))?;
    let CaptionAggregateChange::CaptionAccepted(translation) = update.change else {
        return Err(crate::error::AppError::state(
            "Translation finalization returned the wrong change.",
        ));
    };

    assert_eq!(translation.generation, 12);
    assert_eq!(translation.stream_id, active.stream_id);
    assert_eq!(translation.unit_id.as_deref(), Some("speech-12"));
    assert_eq!(translation.lane, CaptionLane::Translation);
    assert_eq!(translation.revision, 1);
    assert_eq!(translation.text, "private translation");
    assert_eq!(translation.state, CaptionState::Completed);
    assert_eq!(translation.language.as_deref(), Some("zh-Hans"));
    assert_eq!(translation.unit_started_at_ms, Some(100));
    assert_eq!(translation.timestamp_ms, 150);
    assert_eq!(
        translation.source_ref,
        Some(SourceSnapshotRef {
            generation: 12,
            stream_id: active.stream_id,
            unit_id: "speech-12".to_string(),
            revision: 3,
        })
    );

    Ok(())
}

#[test]
fn dropped_reservation_records_stopped_before_releasing_its_source() -> crate::error::AppResult<()>
{
    let store = CaptionAggregateStore::default();
    let active = store
        .begin_generation(13)?
        .active_stream
        .ok_or_else(|| crate::error::AppError::state("Generation 13 was not active."))?;
    store.start_unit(13, &active.stream_id, "stopped".to_string(), 0)?;
    let source = source_caption(
        13,
        &active.stream_id,
        "stopped",
        "private source",
        CaptionState::Completed,
        0,
    );
    let (_, reservation) = store
        .accept_completed_source_for_translation(source.clone())?
        .ok_or_else(|| crate::error::AppError::state("Source was not reserved."))?;

    drop(reservation);

    let snapshot = store.snapshot()?;
    assert!(snapshot.captions.contains(&source));
    assert!(matches!(
        snapshot.translation_units.as_slice(),
        [TranslationUnitSnapshot::Failed {
            reason_code: TranslationFailureReason::Stopped,
            ..
        }]
    ));
    Ok(())
}

#[test]
fn generic_caption_admission_cannot_bypass_a_completed_source_reservation()
-> crate::error::AppResult<()> {
    let store = CaptionAggregateStore::default();
    let active = store
        .begin_generation(13)?
        .active_stream
        .ok_or_else(|| crate::error::AppError::state("Generation 13 was not active."))?;
    store.start_unit(13, &active.stream_id, "speech-13".to_string(), 10)?;
    let source = source_caption(
        13,
        &active.stream_id,
        "speech-13",
        "source",
        CaptionState::Completed,
        10,
    );
    let (_, _reservation) = store
        .accept_completed_source_for_translation(source.clone())?
        .ok_or_else(|| crate::error::AppError::state("Source was not reserved."))?;

    assert!(
        store
            .accept_caption(translation_caption(
                &source,
                "bypass",
                CaptionState::Completed,
            )?)?
            .is_none()
    );

    Ok(())
}

#[test]
fn closing_or_replacing_a_generation_invalidates_its_reservations_and_retrims_history()
-> crate::error::AppResult<()> {
    for replace_generation in [false, true] {
        let store = CaptionAggregateStore::default();
        let active = store
            .begin_generation(14)?
            .active_stream
            .ok_or_else(|| crate::error::AppError::state("Generation 14 was not active."))?;
        for index in 0_u64..6 {
            store.start_unit(14, &active.stream_id, format!("unit-{index}"), index * 10)?;
        }
        let (_, reservation) = store
            .accept_completed_source_for_translation(source_caption(
                14,
                &active.stream_id,
                "unit-0",
                "stale source",
                CaptionState::Completed,
                0,
            ))?
            .ok_or_else(|| crate::error::AppError::state("Source was not reserved."))?;
        for index in 1_u64..6 {
            store.accept_caption(source_caption(
                14,
                &active.stream_id,
                &format!("unit-{index}"),
                format!("caption {index}"),
                CaptionState::Completed,
                index * 10,
            ))?;
        }
        assert_eq!(store.snapshot()?.captions.len(), COMPLETED_UNIT_LIMIT + 1);

        if replace_generation {
            store.begin_generation(15)?;
        } else {
            store.close_generation(14)?;
        }

        let stopped = store.snapshot()?;
        assert_eq!(stopped.captions.len(), COMPLETED_UNIT_LIMIT);
        assert!(stopped.captions.iter().any(|caption| {
            caption.unit_id.as_deref() == Some("unit-0") && caption.text == "stale source"
        }));
        assert!(stopped.translation_units.iter().any(|unit| matches!(
            unit,
            TranslationUnitSnapshot::Failed {
                reason_code: TranslationFailureReason::Stopped,
                source_ref,
            } if source_ref.generation == 14 && source_ref.unit_id == "unit-0"
        )));
        assert!(
            reservation
                .complete_translation("late".to_string(), "zh-Hans".to_string(), 100)?
                .is_none()
        );
        assert!(
            store
                .snapshot()?
                .captions
                .iter()
                .all(|caption| caption.text != "late")
        );
        assert!(
            store
                .snapshot()?
                .translation_units
                .iter()
                .any(|unit| matches!(
                    unit,
                    TranslationUnitSnapshot::Failed {
                        reason_code: TranslationFailureReason::Stopped,
                        source_ref,
                    } if source_ref.generation == 14 && source_ref.unit_id == "unit-0"
                ))
        );
    }

    Ok(())
}
