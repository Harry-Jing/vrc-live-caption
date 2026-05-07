use std::time::{Duration, Instant};

pub(crate) struct SpeechSegmenter {
    sample_rate: u32,
    rms_threshold: f32,
    silence_timeout: Duration,
    min_samples: usize,
    max_samples: usize,
    samples: Vec<f32>,
    active: bool,
    last_voice_at: Option<Instant>,
}

pub(crate) struct SegmenterUpdate {
    pub(crate) speech_started: bool,
    pub(crate) ready_segment: Option<Vec<f32>>,
}

impl SpeechSegmenter {
    pub(crate) fn new(
        sample_rate: u32,
        rms_threshold: f32,
        silence_timeout: Duration,
        min_segment_seconds: f32,
        max_segment_seconds: f32,
    ) -> Self {
        let min_samples = (sample_rate as f32 * min_segment_seconds) as usize;
        let max_samples = (sample_rate as f32 * max_segment_seconds) as usize;

        Self {
            sample_rate,
            rms_threshold,
            silence_timeout,
            min_samples,
            max_samples,
            samples: Vec::with_capacity(max_samples),
            active: false,
            last_voice_at: None,
        }
    }

    pub(crate) fn push_samples(&mut self, samples: Vec<f32>, now: Instant) -> SegmenterUpdate {
        let has_voice = rms(&samples) >= self.rms_threshold;
        let mut speech_started = false;

        if has_voice {
            if !self.active {
                self.active = true;
                speech_started = true;
            }

            self.last_voice_at = Some(now);
        }

        if self.active {
            self.samples.extend(samples);
        }

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
        if self.samples.len() >= self.min_samples {
            self.take_ready_segment()
        } else {
            self.reset();
            None
        }
    }

    pub(crate) fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn ready_after_silence(&mut self, now: Instant) -> Option<Vec<f32>> {
        let silence_elapsed = self
            .last_voice_at
            .map(|last_voice_at| now.duration_since(last_voice_at) >= self.silence_timeout)
            .unwrap_or(false);

        if self.samples.len() >= self.min_samples && silence_elapsed {
            return self.take_ready_segment();
        }

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

    fn reset(&mut self) {
        self.active = false;
        self.last_voice_at = None;
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

    #[test]
    fn starts_speech_when_rms_crosses_threshold() {
        let mut segmenter = SpeechSegmenter::new(10, 0.1, Duration::from_millis(100), 0.2, 1.0);
        let now = Instant::now();
        let update = segmenter.push_samples(vec![0.2, 0.2], now);

        assert!(update.speech_started);
        assert!(update.ready_segment.is_none());
    }

    #[test]
    fn flushes_ready_segment_after_silence() {
        let mut segmenter = SpeechSegmenter::new(10, 0.1, Duration::from_millis(100), 0.2, 1.0);
        let now = Instant::now();

        segmenter.push_samples(vec![0.2, 0.2], now);
        let segment = segmenter.tick(now + Duration::from_millis(120));

        assert_eq!(segment, Some(vec![0.2, 0.2]));
    }

    #[test]
    fn drops_too_short_segment_on_finish() {
        let mut segmenter = SpeechSegmenter::new(10, 0.1, Duration::from_millis(100), 0.3, 1.0);
        let now = Instant::now();

        segmenter.push_samples(vec![0.2, 0.2], now);

        assert_eq!(segmenter.finish(), None);
    }

    #[test]
    fn forces_segment_at_max_duration() {
        let mut segmenter = SpeechSegmenter::new(10, 0.1, Duration::from_millis(100), 0.2, 0.3);
        let now = Instant::now();
        let update = segmenter.push_samples(vec![0.2, 0.2, 0.2], now);

        assert_eq!(update.ready_segment, Some(vec![0.2, 0.2, 0.2]));
    }
}
