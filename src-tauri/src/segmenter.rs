//! Speech segmentation for mono microphone samples.
//!
//! `SpeechSegmenter` owns the VAD threshold, silence timeout, voiced-duration
//! minimum, max segment duration, and pre-roll rules. It is deliberately
//! independent of Tauri, CPAL, STT providers, and OSC so the capture framing
//! behavior can be tested without live devices or network calls.
//!
//! Only *voiced* audio counts toward the minimum segment duration; trailing
//! silence does not. Buffers that never reach the voiced minimum before the
//! silence timeout are discarded as noise blips instead of being padded with
//! silence and uploaded to STT. A short pre-roll captured just before voice
//! onset is prepended to each segment so quiet first syllables survive the
//! RMS gate.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub(crate) struct SpeechSegmenter {
    sample_rate: u32,
    rms_threshold: f32,
    silence_timeout: Duration,
    min_voiced_samples: usize,
    max_samples: usize,
    max_preroll_samples: usize,
    samples: Vec<f32>,
    preroll: VecDeque<f32>,
    voiced_samples: usize,
    active: bool,
    last_voice_at: Option<Instant>,
}

pub(crate) struct SegmenterUpdate {
    /// True when buffered voiced audio first reaches the voiced minimum, not
    /// on the first loud chunk. Noise blips below the minimum are never
    /// announced, so every announced utterance later yields a segment.
    pub(crate) speech_started: bool,
    pub(crate) ready_segment: Option<Vec<f32>>,
}

impl SpeechSegmenter {
    pub(crate) fn new(
        sample_rate: u32,
        rms_threshold: f32,
        silence_timeout: Duration,
        min_voiced_seconds: f32,
        max_segment_seconds: f32,
        preroll_seconds: f32,
    ) -> Self {
        let min_voiced_samples = ((sample_rate as f32 * min_voiced_seconds) as usize).max(1);
        let max_samples = (sample_rate as f32 * max_segment_seconds) as usize;
        let max_preroll_samples = (sample_rate as f32 * preroll_seconds) as usize;

        Self {
            sample_rate,
            rms_threshold,
            silence_timeout,
            min_voiced_samples,
            max_samples,
            max_preroll_samples,
            samples: Vec::with_capacity(max_samples),
            preroll: VecDeque::with_capacity(max_preroll_samples),
            voiced_samples: 0,
            active: false,
            last_voice_at: None,
        }
    }

    pub(crate) fn push_samples(&mut self, samples: Vec<f32>, now: Instant) -> SegmenterUpdate {
        let has_voice = rms(&samples) >= self.rms_threshold;
        let voiced_before = self.voiced_samples;

        if has_voice {
            if !self.active {
                self.active = true;
                self.samples.extend(self.preroll.drain(..));
            }

            self.last_voice_at = Some(now);
            self.voiced_samples += samples.len();
        }

        if self.active {
            self.samples.extend(samples);
        } else {
            self.push_preroll(samples);
        }

        let speech_started = voiced_before < self.min_voiced_samples
            && self.voiced_samples >= self.min_voiced_samples;
        let ready_segment = if self.samples.len() >= self.max_samples {
            self.take_ready_segment()
        } else {
            self.ready_after_silence(now)
        };

        SegmenterUpdate {
            speech_started,
            ready_segment,
        }
    }

    pub(crate) fn tick(&mut self, now: Instant) -> Option<Vec<f32>> {
        self.ready_after_silence(now)
    }

    pub(crate) fn finish(&mut self) -> Option<Vec<f32>> {
        if self.voiced_samples >= self.min_voiced_samples {
            self.take_ready_segment()
        } else {
            self.discard_buffer();
            None
        }
    }

    pub(crate) fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn push_preroll(&mut self, samples: Vec<f32>) {
        if self.max_preroll_samples == 0 {
            return;
        }

        self.preroll.extend(samples);

        let excess = self.preroll.len().saturating_sub(self.max_preroll_samples);

        if excess > 0 {
            self.preroll.drain(..excess);
        }
    }

    fn ready_after_silence(&mut self, now: Instant) -> Option<Vec<f32>> {
        let silence_elapsed = self
            .last_voice_at
            .map(|last_voice_at| now.duration_since(last_voice_at) >= self.silence_timeout)
            .unwrap_or(false);

        if !silence_elapsed {
            return None;
        }

        if self.voiced_samples >= self.min_voiced_samples {
            return self.take_ready_segment();
        }

        // The buffer went quiet before reaching the voiced minimum: this was
        // a noise blip, not speech, so it must not reach STT.
        self.discard_buffer();

        None
    }

    fn take_ready_segment(&mut self) -> Option<Vec<f32>> {
        if self.samples.is_empty() {
            self.reset();
            return None;
        }

        let samples = std::mem::take(&mut self.samples);
        self.reset();

        Some(samples)
    }

    fn discard_buffer(&mut self) {
        self.samples.clear();
        self.reset();
    }

    fn reset(&mut self) {
        self.active = false;
        self.last_voice_at = None;
        self.voiced_samples = 0;
        self.preroll.clear();
    }
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }

    let sum_of_squares = samples.iter().map(|sample| sample * sample).sum::<f32>();

    (sum_of_squares / samples.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
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
}
