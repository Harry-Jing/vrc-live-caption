use super::*;

#[test]
fn shared_v1_fixture_round_trips_without_a_stable_state() -> Result<(), serde_json::Error> {
    let fixture = include_str!("../../contracts/caption-session-snapshot-v1.json");
    let expected = serde_json::from_str::<serde_json::Value>(fixture)?;
    let snapshot = serde_json::from_str::<CaptionSessionSnapshotV1>(fixture)?;
    let actual = serde_json::to_value(snapshot)?;

    assert_eq!(actual, expected);
    assert!(!fixture.contains("stable"));

    Ok(())
}

#[test]
fn bounded_completion_produces_the_shared_v1_session_snapshot() -> crate::error::AppResult<()> {
    let store = CaptionSessionStore::default();
    let started = store.begin_generation(7)?;
    let active = started
        .active
        .ok_or_else(|| crate::error::AppError::state("Test generation did not become active."))?;

    assert_eq!(active.stream_id, "recognition-7-1");
    assert!(
        store
            .start_unit(7, &active.stream_id, "speech-7-1".to_string(), 1_000)?
            .is_some()
    );
    assert!(
        store
            .accept_caption(CaptionSnapshotV1 {
                generation: 7,
                stream_id: active.stream_id,
                unit_id: Some("speech-7-1".to_string()),
                lane: CaptionLane::Source,
                revision: 1,
                text: "Full bounded OpenAI transcript.".to_string(),
                state: CaptionState::Completed,
                language: Some("en".to_string()),
                provider: "openai".to_string(),
                model: "gpt-transcribe".to_string(),
                unit_started_at_ms: Some(1_000),
                timestamp_ms: 1_200,
            })?
            .is_some()
    );

    let expected = serde_json::from_str::<serde_json::Value>(include_str!(
        "../../contracts/caption-session-snapshot-v1.json"
    ))
    .map_err(|error| crate::error::AppError::state(error.to_string()))?;
    let actual = serde_json::to_value(store.snapshot()?)
        .map_err(|error| crate::error::AppError::state(error.to_string()))?;

    assert_eq!(actual, expected);

    Ok(())
}

#[test]
fn stop_and_new_generation_fence_old_writers_without_losing_completed_history()
-> crate::error::AppResult<()> {
    let store = CaptionSessionStore::default();
    let active = store
        .begin_generation(1)?
        .active
        .ok_or_else(|| crate::error::AppError::state("Generation 1 was not active."))?;
    store.start_unit(1, &active.stream_id, "completed".to_string(), 10)?;
    let completed = CaptionSnapshotV1 {
        generation: 1,
        stream_id: active.stream_id.clone(),
        unit_id: Some("completed".to_string()),
        lane: CaptionLane::Source,
        revision: 1,
        text: "kept".to_string(),
        state: CaptionState::Completed,
        language: Some("en".to_string()),
        provider: "openai".to_string(),
        model: "gpt-transcribe".to_string(),
        unit_started_at_ms: Some(10),
        timestamp_ms: 20,
    };
    assert!(store.accept_caption(completed.clone())?.is_some());
    assert!(
        store
            .accept_caption(CaptionSnapshotV1 {
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
            .accept_caption(CaptionSnapshotV1 {
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

    assert!(stopped.active.is_none());
    assert!(stopped.active_units.is_empty());
    assert_eq!(stopped.captions.len(), 1);
    assert_eq!(stopped.captions[0].text, "kept");
    assert!(store.accept_caption(completed.clone())?.is_none());

    let next = store.begin_generation(2)?;
    assert_eq!(
        next.active.as_ref().map(|active| active.stream_id.as_str()),
        Some("recognition-2-1")
    );
    assert!(
        store
            .start_unit(1, &active.stream_id, "late".to_string(), 50)?
            .is_none()
    );
    assert_eq!(store.begin_generation(1)?, next);

    let next_active = next
        .active
        .ok_or_else(|| crate::error::AppError::state("Generation 2 was not active."))?;
    store.start_unit(2, &next_active.stream_id, "no-result".to_string(), 60)?;
    assert!(
        store
            .end_unit_without_caption(2, &next_active.stream_id, "no-result")?
            .is_some()
    );
    assert!(
        store
            .start_unit(2, &next_active.stream_id, "no-result".to_string(), 60,)?
            .is_none()
    );
    assert!(
        store
            .accept_caption(CaptionSnapshotV1 {
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
    let store = CaptionSessionStore::default();
    let active = store
        .begin_generation(3)?
        .active
        .ok_or_else(|| crate::error::AppError::state("Generation 3 was not active."))?;
    let first = CaptionSnapshotV1 {
        generation: 3,
        stream_id: active.stream_id,
        unit_id: None,
        lane: CaptionLane::Source,
        revision: 1,
        text: "first full snapshot".to_string(),
        state: CaptionState::Ongoing,
        language: Some("en".to_string()),
        provider: "streaming-provider".to_string(),
        model: "streaming-model".to_string(),
        unit_started_at_ms: None,
        timestamp_ms: 10,
    };

    assert!(store.accept_caption(first.clone())?.is_some());
    assert!(store.accept_caption(first.clone())?.is_none());
    assert!(
        store
            .accept_caption(CaptionSnapshotV1 {
                revision: 0,
                text: "older".to_string(),
                ..first.clone()
            })?
            .is_none()
    );
    assert!(
        store
            .accept_caption(CaptionSnapshotV1 {
                revision: 2,
                text: "replacement full snapshot".to_string(),
                timestamp_ms: 20,
                ..first.clone()
            })?
            .is_some()
    );
    assert!(
        store
            .accept_caption(CaptionSnapshotV1 {
                revision: 1,
                text: "stale full snapshot".to_string(),
                timestamp_ms: 25,
                ..first.clone()
            })?
            .is_none()
    );
    assert!(
        store
            .accept_caption(CaptionSnapshotV1 {
                revision: 3,
                text: "unitless completion is invalid".to_string(),
                state: CaptionState::Completed,
                timestamp_ms: 30,
                ..first
            })?
            .is_none()
    );

    let snapshot = store.snapshot()?;
    assert!(snapshot.active_units.is_empty());
    assert_eq!(snapshot.captions.len(), 1);
    assert_eq!(snapshot.captions[0].revision, 2);
    assert_eq!(snapshot.captions[0].text, "replacement full snapshot");
    assert_eq!(snapshot.captions[0].state, CaptionState::Ongoing);

    Ok(())
}

#[test]
fn completed_history_keeps_the_five_newest_units_in_newest_first_order()
-> crate::error::AppResult<()> {
    let store = CaptionSessionStore::default();
    let active = store
        .begin_generation(4)?
        .active
        .ok_or_else(|| crate::error::AppError::state("Generation 4 was not active."))?;

    for index in 0_u64..7 {
        let unit_id = format!("unit-{index}");
        store.start_unit(4, &active.stream_id, unit_id.clone(), index * 10)?;
        assert!(
            store
                .accept_caption(CaptionSnapshotV1 {
                    generation: 4,
                    stream_id: active.stream_id.clone(),
                    unit_id: Some(unit_id),
                    lane: CaptionLane::Source,
                    revision: 1,
                    text: format!("caption {index}"),
                    state: CaptionState::Completed,
                    language: Some("en".to_string()),
                    provider: "openai".to_string(),
                    model: "gpt-transcribe".to_string(),
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
fn terminal_unit_replay_guard_stays_bounded_during_a_long_generation() -> crate::error::AppResult<()>
{
    let store = CaptionSessionStore::default();
    let active = store
        .begin_generation(5)?
        .active
        .ok_or_else(|| crate::error::AppError::state("Generation 5 was not active."))?;
    let expected_replay_bound = TERMINAL_UNIT_REPLAY_LIMIT;
    let retained_unit_id = "retained-completed".to_string();
    store.start_unit(5, &active.stream_id, retained_unit_id.clone(), 0)?;
    assert!(
        store
            .accept_caption(CaptionSnapshotV1 {
                generation: 5,
                stream_id: active.stream_id.clone(),
                unit_id: Some(retained_unit_id.clone()),
                lane: CaptionLane::Source,
                revision: 1,
                text: "retained".to_string(),
                state: CaptionState::Completed,
                language: Some("en".to_string()),
                provider: "openai".to_string(),
                model: "gpt-transcribe".to_string(),
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
                .end_unit_without_caption(5, &active.stream_id, &unit_id)?
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
    assert_eq!(state.recent_terminal_units.len(), expected_replay_bound);

    Ok(())
}
