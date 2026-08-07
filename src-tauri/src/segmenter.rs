//! Streaming application-owned speech boundaries for mono microphone audio.
//!
//! A short candidate is buffered until it reaches the voiced minimum. Once it
//! is accepted, the buffered pre-roll and every later frame are released
//! immediately so a streaming recognizer can emit text before the unit ends.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub(crate) struct SpeechSegmenter {
    sample_rate: u32,
    rms_threshold: f32,
    silence_timeout: Duration,
    min_voiced_samples: usize,
    max_samples: usize,
    max_preroll_samples: usize,
    candidate_audio: Vec<f32>,
    preroll: VecDeque<f32>,
    voiced_samples: usize,
    active_samples: usize,
    candidate_active: bool,
    speech_announced: bool,
    last_voice_at: Option<Instant>,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct SegmenterUpdate {
    /// The buffered candidate has crossed the voiced minimum and now owns a
    /// recognition unit. `audio` contains its pre-roll and candidate frames.
    pub(crate) speech_started: bool,
    /// Audio that belongs to the announced unit. Empty before a candidate is
    /// accepted and after it has ended.
    pub(crate) audio: Vec<f32>,
    /// The announced unit reached silence or the maximum duration and must be
    /// committed after `audio` is appended.
    pub(crate) speech_ended: bool,
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
        let max_samples = ((sample_rate as f32 * max_segment_seconds) as usize).max(1);
        let max_preroll_samples = (sample_rate as f32 * preroll_seconds) as usize;

        Self {
            sample_rate,
            rms_threshold,
            silence_timeout,
            min_voiced_samples,
            max_samples,
            max_preroll_samples,
            candidate_audio: Vec::with_capacity(min_voiced_samples),
            preroll: VecDeque::with_capacity(max_preroll_samples),
            voiced_samples: 0,
            active_samples: 0,
            candidate_active: false,
            speech_announced: false,
            last_voice_at: None,
        }
    }

    pub(crate) fn push_samples(&mut self, samples: Vec<f32>, now: Instant) -> SegmenterUpdate {
        if samples.is_empty() {
            return self.tick(now);
        }

        let has_voice = rms(&samples) >= self.rms_threshold;
        if has_voice && !self.candidate_active {
            self.candidate_active = true;
            self.candidate_audio.extend(self.preroll.drain(..));
            self.active_samples = self.candidate_audio.len();
        }

        if !self.candidate_active {
            self.push_preroll(samples);
            return SegmenterUpdate::default();
        }

        if has_voice {
            self.last_voice_at = Some(now);
            self.voiced_samples = self.voiced_samples.saturating_add(samples.len());
        }
        self.active_samples = self.active_samples.saturating_add(samples.len());

        let mut update = if self.speech_announced {
            SegmenterUpdate {
                audio: samples,
                ..SegmenterUpdate::default()
            }
        } else {
            self.candidate_audio.extend(samples);
            if self.voiced_samples >= self.min_voiced_samples {
                self.speech_announced = true;
                SegmenterUpdate {
                    speech_started: true,
                    audio: std::mem::take(&mut self.candidate_audio),
                    speech_ended: false,
                }
            } else {
                SegmenterUpdate::default()
            }
        };

        if self.active_samples >= self.max_samples || self.silence_elapsed(now) {
            update.speech_ended = self.speech_announced;
            self.reset_candidate();
        }

        update
    }

    pub(crate) fn tick(&mut self, now: Instant) -> SegmenterUpdate {
        if !self.candidate_active || !self.silence_elapsed(now) {
            return SegmenterUpdate::default();
        }

        let update = SegmenterUpdate {
            speech_ended: self.speech_announced,
            ..SegmenterUpdate::default()
        };
        self.reset_candidate();
        update
    }

    /// Discards an open tail instead of committing it during Stop or failure.
    pub(crate) fn finish(&mut self) -> SegmenterUpdate {
        let update = SegmenterUpdate {
            speech_ended: self.speech_announced,
            ..SegmenterUpdate::default()
        };
        self.reset_candidate();
        update
    }

    pub(crate) fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn silence_elapsed(&self, now: Instant) -> bool {
        self.last_voice_at
            .is_some_and(|last_voice_at| now.duration_since(last_voice_at) >= self.silence_timeout)
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

    fn reset_candidate(&mut self) {
        self.candidate_audio.clear();
        self.preroll.clear();
        self.voiced_samples = 0;
        self.active_samples = 0;
        self.candidate_active = false;
        self.speech_announced = false;
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
#[path = "segmenter_tests.rs"]
mod tests;
