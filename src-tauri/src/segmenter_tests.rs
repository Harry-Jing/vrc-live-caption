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
fn accepts_a_candidate_and_releases_its_audio_immediately() {
    let mut segmenter = segmenter(0.3, 2.0, NO_PREROLL);
    let now = Instant::now();

    assert_eq!(
        segmenter.push_samples(vec![0.2, 0.2], now),
        SegmenterUpdate::default()
    );
    let update = segmenter.push_samples(vec![0.2, 0.2], now + Duration::from_millis(10));

    assert!(update.speech_started);
    assert_eq!(update.audio, vec![0.2, 0.2, 0.2, 0.2]);
    assert!(!update.speech_ended);
}

#[test]
fn streams_later_frames_without_waiting_for_the_boundary() {
    let mut segmenter = segmenter(0.2, 2.0, NO_PREROLL);
    let now = Instant::now();
    segmenter.push_samples(vec![0.2, 0.2], now);

    let update = segmenter.push_samples(vec![0.3, 0.3], now + Duration::from_millis(10));

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

    let update = segmenter.push_samples(vec![0.2, 0.2], now + Duration::from_millis(20));

    assert!(update.speech_started);
    assert_eq!(update.audio, vec![0.03, 0.04, 0.2, 0.2]);
}

#[test]
fn max_duration_commits_an_announced_unit_without_losing_its_audio() {
    let mut segmenter = segmenter(0.2, 0.3, NO_PREROLL);

    let update = segmenter.push_samples(vec![0.2, 0.2, 0.2], Instant::now());

    assert!(update.speech_started);
    assert_eq!(update.audio, vec![0.2, 0.2, 0.2]);
    assert!(update.speech_ended);
}

#[test]
fn finish_marks_only_an_announced_tail_as_discarded() {
    let mut accepted = segmenter(0.2, 2.0, NO_PREROLL);
    accepted.push_samples(vec![0.2, 0.2], Instant::now());
    assert!(accepted.finish().speech_ended);

    let mut candidate = segmenter(0.3, 2.0, NO_PREROLL);
    candidate.push_samples(vec![0.2, 0.2], Instant::now());
    assert_eq!(candidate.finish(), SegmenterUpdate::default());
}
