use super::*;

const NO_PREROLL: f32 = 0.0;

fn segmenter(
    min_voiced_seconds: f32,
    max_segment_seconds: f32,
    preroll_seconds: f32,
) -> SpeechSegmenter {
    SpeechSegmenter::new(
        10,
        0.1,
        Duration::from_millis(100),
        min_voiced_seconds,
        max_segment_seconds,
        preroll_seconds,
    )
}

fn only_update(mut updates: Vec<SegmenterUpdate>) -> SegmenterUpdate {
    assert_eq!(updates.len(), 1);
    updates.pop().unwrap_or_default()
}

#[test]
fn accepts_a_candidate_and_releases_its_audio_immediately() {
    let mut segmenter = segmenter(0.3, 2.0, NO_PREROLL);
    let now = Instant::now();

    assert!(segmenter.push_samples(vec![0.2, 0.2], now).is_empty());
    let update =
        only_update(segmenter.push_samples(vec![0.2, 0.2], now + Duration::from_millis(10)));

    assert!(update.speech_started);
    assert_eq!(update.audio, vec![0.2, 0.2, 0.2, 0.2]);
    assert!(!update.speech_ended);
}

#[test]
fn streams_later_frames_without_waiting_for_the_boundary() {
    let mut segmenter = segmenter(0.2, 2.0, NO_PREROLL);
    let now = Instant::now();
    segmenter.push_samples(vec![0.2, 0.2], now);

    let update =
        only_update(segmenter.push_samples(vec![0.3, 0.3], now + Duration::from_millis(10)));

    assert!(!update.speech_started);
    assert_eq!(update.audio, vec![0.3, 0.3]);
    assert!(!update.speech_ended);
}

#[test]
fn ends_an_announced_unit_after_silence() {
    let mut segmenter = segmenter(0.2, 2.0, NO_PREROLL);
    let now = Instant::now();
    segmenter.push_samples(vec![0.2, 0.2], now);

    let update = segmenter.tick(now + Duration::from_millis(120));

    assert!(update.speech_ended);
    assert!(update.audio.is_empty());
}

#[test]
fn discards_a_noise_blip_without_announcing_or_ending_a_unit() {
    let mut segmenter = segmenter(0.3, 2.0, NO_PREROLL);
    let now = Instant::now();
    segmenter.push_samples(vec![0.2, 0.2], now);

    assert_eq!(
        segmenter.tick(now + Duration::from_millis(120)),
        SegmenterUpdate::default()
    );
}

#[test]
fn prepends_only_the_capped_preroll_when_speech_is_accepted() {
    let mut segmenter = segmenter(0.2, 2.0, 0.2);
    let now = Instant::now();
    segmenter.push_samples(vec![0.01, 0.02], now);
    segmenter.push_samples(vec![0.03, 0.04], now + Duration::from_millis(10));

    let update =
        only_update(segmenter.push_samples(vec![0.2, 0.2], now + Duration::from_millis(20)));

    assert!(update.speech_started);
    assert_eq!(update.audio, vec![0.03, 0.04, 0.2, 0.2]);
}

#[test]
fn max_duration_commits_an_announced_unit_without_losing_its_audio() {
    let mut segmenter = segmenter(0.2, 0.3, NO_PREROLL);

    let update = only_update(segmenter.push_samples(vec![0.2, 0.2, 0.2], Instant::now()));

    assert!(update.speech_started);
    assert_eq!(update.audio, vec![0.2, 0.2, 0.2]);
    assert!(update.speech_ended);
}

#[test]
fn splits_one_voiced_callback_at_the_exact_max_without_losing_or_repeating_samples() {
    let mut segmenter = segmenter(0.1, 0.3, NO_PREROLL);

    let updates = segmenter.push_samples(vec![0.21, 0.22, 0.23, 0.24, 0.25], Instant::now());

    assert_eq!(updates.len(), 2);
    assert_eq!(
        updates[0],
        SegmenterUpdate {
            speech_started: true,
            audio: vec![0.21, 0.22, 0.23],
            speech_ended: true,
        }
    );
    assert_eq!(
        updates[1],
        SegmenterUpdate {
            speech_started: true,
            audio: vec![0.24, 0.25],
            speech_ended: false,
        }
    );
}

#[test]
fn confirmed_short_tail_after_the_max_starts_immediately_and_ends_after_silence() {
    let mut segmenter = segmenter(0.3, 0.3, NO_PREROLL);
    let now = Instant::now();

    let updates = segmenter.push_samples(vec![0.21, 0.22, 0.23, 0.24], now);

    assert_eq!(updates.len(), 2);
    assert_eq!(
        updates[0],
        SegmenterUpdate {
            speech_started: true,
            audio: vec![0.21, 0.22, 0.23],
            speech_ended: true,
        }
    );
    assert_eq!(
        updates[1],
        SegmenterUpdate {
            speech_started: true,
            audio: vec![0.24],
            speech_ended: false,
        }
    );
    assert!(
        segmenter
            .tick(now + Duration::from_millis(120))
            .speech_ended
    );
}

#[test]
fn voiced_callback_after_an_exact_max_boundary_starts_as_a_continuation() {
    let mut segmenter = segmenter(0.3, 0.3, NO_PREROLL);
    let now = Instant::now();
    let boundary = only_update(segmenter.push_samples(vec![0.21, 0.22, 0.23], now));
    assert!(boundary.speech_ended);

    let continuation =
        only_update(segmenter.push_samples(vec![0.24], now + Duration::from_millis(50)));

    assert!(continuation.speech_started);
    assert_eq!(continuation.audio, vec![0.24]);
    assert!(!continuation.speech_ended);
}

#[test]
fn trusted_continuation_keeps_silent_preroll_with_the_next_short_voice() {
    let mut segmenter = segmenter(0.3, 0.3, 0.2);
    let now = Instant::now();
    let boundary = only_update(segmenter.push_samples(vec![0.21, 0.22, 0.23], now));
    assert!(boundary.speech_ended);
    assert!(
        segmenter
            .push_samples(vec![0.01, 0.02], now + Duration::from_millis(20))
            .is_empty()
    );

    let continuation =
        only_update(segmenter.push_samples(vec![0.24], now + Duration::from_millis(50)));

    assert_eq!(
        continuation,
        SegmenterUpdate {
            speech_started: true,
            audio: vec![0.01, 0.02, 0.24],
            speech_ended: true,
        }
    );
}

#[test]
fn silence_timeout_after_a_max_boundary_restores_the_minimum_voice_gate() {
    let mut segmenter = segmenter(0.3, 0.3, NO_PREROLL);
    let now = Instant::now();
    let boundary = only_update(segmenter.push_samples(vec![0.21, 0.22, 0.23], now));
    assert!(boundary.speech_ended);

    assert_eq!(
        segmenter.tick(now + Duration::from_millis(120)),
        SegmenterUpdate::default()
    );
    assert!(
        segmenter
            .push_samples(vec![0.24], now + Duration::from_millis(130))
            .is_empty()
    );

    let new_speech =
        only_update(segmenter.push_samples(vec![0.25, 0.26], now + Duration::from_millis(140)));
    assert!(new_speech.speech_started);
    assert_eq!(new_speech.audio, vec![0.24, 0.25, 0.26]);
    assert!(new_speech.speech_ended);
}

#[test]
fn silent_samples_that_reach_the_max_do_not_confirm_a_continuation() {
    let mut segmenter = segmenter(0.2, 0.3, NO_PREROLL);
    let now = Instant::now();
    let started = only_update(segmenter.push_samples(vec![0.21, 0.22], now));
    assert!(!started.speech_ended);

    let ended = only_update(segmenter.push_samples(vec![0.01], now + Duration::from_millis(10)));
    assert!(ended.speech_ended);
    assert!(
        segmenter
            .push_samples(vec![0.23], now + Duration::from_millis(20))
            .is_empty()
    );

    let new_speech =
        only_update(segmenter.push_samples(vec![0.24], now + Duration::from_millis(30)));
    assert!(new_speech.speech_started);
    assert_eq!(new_speech.audio, vec![0.23, 0.24]);
}

#[test]
fn counts_preroll_toward_the_exact_max_and_processes_the_callback_remainder() {
    let mut segmenter = segmenter(0.1, 0.3, 0.1);
    let now = Instant::now();
    assert!(segmenter.push_samples(vec![0.01], now).is_empty());

    let updates = segmenter.push_samples(
        vec![0.21, 0.22, 0.23, 0.24],
        now + Duration::from_millis(10),
    );

    assert_eq!(updates.len(), 2);
    assert_eq!(updates[0].audio, vec![0.01, 0.21, 0.22]);
    assert!(updates[0].speech_started);
    assert!(updates[0].speech_ended);
    assert_eq!(updates[1].audio, vec![0.23, 0.24]);
    assert!(updates[1].speech_started);
    assert!(!updates[1].speech_ended);
}

#[test]
fn one_callback_can_cross_multiple_exact_max_boundaries() {
    let mut segmenter = segmenter(0.1, 0.3, NO_PREROLL);
    let input = vec![0.21, 0.22, 0.23, 0.24, 0.25, 0.26, 0.27, 0.28];

    let updates = segmenter.push_samples(input.clone(), Instant::now());

    assert_eq!(updates.len(), 3);
    assert_eq!(updates[0].audio, vec![0.21, 0.22, 0.23]);
    assert!(updates[0].speech_ended);
    assert_eq!(updates[1].audio, vec![0.24, 0.25, 0.26]);
    assert!(updates[1].speech_ended);
    assert_eq!(updates[2].audio, vec![0.27, 0.28]);
    assert!(!updates[2].speech_ended);
    assert_eq!(
        updates
            .iter()
            .flat_map(|update| update.audio.iter().copied())
            .collect::<Vec<_>>(),
        input
    );
}

#[test]
fn silent_remainder_after_the_max_becomes_preroll_for_later_voice() {
    let mut segmenter = segmenter(0.1, 0.3, 0.2);
    let now = Instant::now();
    let started = only_update(segmenter.push_samples(vec![0.21, 0.22], now));
    assert_eq!(started.audio, vec![0.21, 0.22]);

    let ended = only_update(
        segmenter.push_samples(vec![0.01, 0.02, 0.03], now + Duration::from_millis(10)),
    );
    assert_eq!(ended.audio, vec![0.01]);
    assert!(ended.speech_ended);

    let next = only_update(segmenter.push_samples(vec![0.23], now + Duration::from_millis(20)));
    assert_eq!(next.audio, vec![0.02, 0.03, 0.23]);
    assert!(next.speech_started);
    assert!(next.speech_ended);
}

#[test]
fn finish_marks_only_an_announced_tail_as_discarded() {
    let mut accepted = segmenter(0.2, 2.0, NO_PREROLL);
    accepted.push_samples(vec![0.2, 0.2], Instant::now());
    assert!(accepted.finish().speech_ended);

    let mut candidate = segmenter(0.3, 2.0, NO_PREROLL);
    candidate.push_samples(vec![0.2, 0.2], Instant::now());
    assert_eq!(candidate.finish(), SegmenterUpdate::default());

    let mut pending_continuation = segmenter(0.3, 0.3, NO_PREROLL);
    let now = Instant::now();
    pending_continuation.push_samples(vec![0.2, 0.2, 0.2], now);
    assert_eq!(pending_continuation.finish(), SegmenterUpdate::default());
    assert!(
        pending_continuation
            .push_samples(vec![0.3], now + Duration::from_millis(10))
            .is_empty()
    );
}
