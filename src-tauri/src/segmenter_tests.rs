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

#[test]
fn starts_speech_when_voiced_minimum_is_reached() {
    let mut segmenter = segmenter(0.2, 1.0, NO_PREROLL);
    let update = segmenter.push_samples(vec![0.2, 0.2], Instant::now());

    assert!(update.speech_started);
    assert!(update.ready_segment.is_none());
}

#[test]
fn speech_start_waits_for_voiced_minimum() {
    let mut segmenter = segmenter(0.3, 1.0, NO_PREROLL);
    let now = Instant::now();

    let first = segmenter.push_samples(vec![0.2, 0.2], now);
    assert!(!first.speech_started);

    let second = segmenter.push_samples(vec![0.2, 0.2], now + Duration::from_millis(10));
    assert!(second.speech_started);
}

#[test]
fn flushes_ready_segment_after_silence() {
    let mut segmenter = segmenter(0.2, 1.0, NO_PREROLL);
    let now = Instant::now();

    segmenter.push_samples(vec![0.2, 0.2], now);
    let segment = segmenter.tick(now + Duration::from_millis(120));

    assert_eq!(segment, Some(vec![0.2, 0.2]));
}

#[test]
fn discards_noise_blip_after_silence_timeout() {
    let mut segmenter = segmenter(0.3, 1.0, NO_PREROLL);
    let now = Instant::now();

    let blip = segmenter.push_samples(vec![0.2, 0.2], now);
    assert!(!blip.speech_started);
    assert_eq!(segmenter.tick(now + Duration::from_millis(120)), None);

    // The discarded blip must not leak into the next utterance.
    let later = now + Duration::from_millis(500);
    let update = segmenter.push_samples(vec![0.3, 0.3, 0.3], later);
    assert!(update.speech_started);

    let segment = segmenter.tick(later + Duration::from_millis(120));
    assert_eq!(segment, Some(vec![0.3, 0.3, 0.3]));
}

#[test]
fn prepends_capped_preroll_to_segment() {
    let mut segmenter = segmenter(0.2, 1.0, 0.2);
    let now = Instant::now();

    segmenter.push_samples(vec![0.01, 0.02], now);
    segmenter.push_samples(vec![0.03, 0.04], now + Duration::from_millis(10));

    let update = segmenter.push_samples(vec![0.2, 0.2], now + Duration::from_millis(20));
    assert!(update.speech_started);

    let segment = segmenter.tick(now + Duration::from_millis(200));
    assert_eq!(segment, Some(vec![0.03, 0.04, 0.2, 0.2]));
}

#[test]
fn drops_segment_below_voiced_minimum_on_finish() {
    let mut segmenter = segmenter(0.3, 1.0, NO_PREROLL);

    segmenter.push_samples(vec![0.2, 0.2], Instant::now());

    assert_eq!(segmenter.finish(), None);
}

#[test]
fn finish_returns_tail_with_enough_voiced_audio() {
    let mut segmenter = segmenter(0.2, 1.0, NO_PREROLL);

    segmenter.push_samples(vec![0.2, 0.2], Instant::now());

    assert_eq!(segmenter.finish(), Some(vec![0.2, 0.2]));
}

#[test]
fn forces_segment_at_max_duration() {
    let mut segmenter = segmenter(0.2, 0.3, NO_PREROLL);
    let update = segmenter.push_samples(vec![0.2, 0.2, 0.2], Instant::now());

    assert_eq!(update.ready_segment, Some(vec![0.2, 0.2, 0.2]));
}

#[test]
fn discards_max_duration_buffer_below_voiced_minimum() {
    let mut segmenter = segmenter(0.3, 0.5, NO_PREROLL);
    let now = Instant::now();

    // Sparse clicks keep the buffer active without ever reaching the
    // voiced minimum, so the buffer fills to the max segment duration.
    segmenter.push_samples(vec![0.2, 0.2], now);
    segmenter.push_samples(vec![0.0, 0.0], now + Duration::from_millis(10));
    let update = segmenter.push_samples(vec![0.0], now + Duration::from_millis(20));

    assert!(!update.speech_started);
    assert_eq!(update.ready_segment, None);

    // The discarded noise must not leak into the next utterance.
    let later = now + Duration::from_millis(500);
    let update = segmenter.push_samples(vec![0.3, 0.3, 0.3], later);
    assert!(update.speech_started);
    assert_eq!(
        segmenter.tick(later + Duration::from_millis(120)),
        Some(vec![0.3, 0.3, 0.3])
    );
}
