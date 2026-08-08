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
    /// A hard maximum is a technical split, not evidence that speech stopped.
    /// A voiced continuation before this trust expires skips the new-speech
    /// minimum so a short tail is not discarded.
    confirmed_continuation_at: Option<Instant>,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct SegmenterUpdate {
    /// A buffered candidate has crossed the voiced minimum, or a recently
    /// hard-split continuation has supplied voice. `audio` contains its
    /// pre-roll and candidate frames.
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
            confirmed_continuation_at: None,
        }
    }

    /// Accepts one capture callback and returns every ordered unit transition
    /// caused by it. A callback can cross the hard maximum, so one input may
    /// end the current unit and immediately start one or more confirmed
    /// continuation units without reapplying the new-speech minimum.
    pub(crate) fn push_samples(&mut self, samples: Vec<f32>, now: Instant) -> Vec<SegmenterUpdate> {
        if samples.is_empty() {
            return meaningful_update(self.tick(now)).into_iter().collect();
        }
        self.expire_confirmed_continuation(now);

        let has_voice = rms(&samples) >= self.rms_threshold;
        let mut remaining = samples;
        let mut updates = Vec::new();

        while !remaining.is_empty() {
            let mut continuation_started = false;
            if has_voice && !self.candidate_active {
                self.candidate_active = true;
                self.candidate_audio.extend(self.preroll.drain(..));
                self.active_samples = self.candidate_audio.len();
                if self.confirmed_continuation_at.take().is_some() {
                    continuation_started = true;
                }
            }

            if !self.candidate_active {
                self.push_preroll(remaining);
                break;
            }

            let available_samples = self.max_samples.saturating_sub(self.active_samples);
            if available_samples == 0 {
                if self.speech_announced {
                    updates.push(SegmenterUpdate {
                        speech_ended: true,
                        ..SegmenterUpdate::default()
                    });
                }
                self.reset_candidate();
                continue;
            }

            let tail = if remaining.len() > available_samples {
                remaining.split_off(available_samples)
            } else {
                Vec::new()
            };
            let current = std::mem::replace(&mut remaining, tail);
            let mut update =
                self.push_candidate_samples(current, has_voice, continuation_started, now);

            let reached_max = self.active_samples >= self.max_samples;
            if reached_max || self.silence_elapsed(now) {
                update.speech_ended = self.speech_announced;
                let confirmed_continuation_at = if reached_max && self.speech_announced && has_voice
                {
                    Some(now)
                } else {
                    None
                };
                self.reset_candidate();
                self.confirmed_continuation_at = confirmed_continuation_at;
            }

            if let Some(update) = meaningful_update(update) {
                updates.push(update);
            }
        }

        updates
    }

    pub(crate) fn tick(&mut self, now: Instant) -> SegmenterUpdate {
        self.expire_confirmed_continuation(now);
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

    fn push_candidate_samples(
        &mut self,
        samples: Vec<f32>,
        has_voice: bool,
        force_announce: bool,
        now: Instant,
    ) -> SegmenterUpdate {
        if has_voice {
            self.last_voice_at = Some(now);
            self.voiced_samples = self.voiced_samples.saturating_add(samples.len());
        }
        self.active_samples = self.active_samples.saturating_add(samples.len());

        if self.speech_announced {
            return SegmenterUpdate {
                audio: samples,
                ..SegmenterUpdate::default()
            };
        }

        self.candidate_audio.extend(samples);
        if !force_announce && self.voiced_samples < self.min_voiced_samples {
            return SegmenterUpdate::default();
        }

        self.speech_announced = true;
        SegmenterUpdate {
            speech_started: true,
            audio: std::mem::take(&mut self.candidate_audio),
            speech_ended: false,
        }
    }

    fn expire_confirmed_continuation(&mut self, now: Instant) {
        if self.confirmed_continuation_at.is_some_and(|confirmed_at| {
            now.saturating_duration_since(confirmed_at) >= self.silence_timeout
        }) {
            self.confirmed_continuation_at = None;
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
        self.confirmed_continuation_at = None;
    }
}

fn meaningful_update(update: SegmenterUpdate) -> Option<SegmenterUpdate> {
    (update != SegmenterUpdate::default()).then_some(update)
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
