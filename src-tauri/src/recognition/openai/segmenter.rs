//! Streaming application-owned speech boundaries for mono microphone audio.
//!
//! A short candidate is buffered until it reaches the voiced minimum. Once it
//! is accepted, the buffered pre-roll and every later frame are released
//! immediately so a streaming recognizer can emit text before the unit ends.

use crate::audio_level::VAD_ANALYSIS_FRAME_MILLIS;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub(crate) struct SpeechSegmenter {
    sample_rate: u32,
    rms_threshold: f32,
    silence_timeout_samples: usize,
    analysis_frame_samples: usize,
    analysis_remainder: Vec<f32>,
    min_voiced_samples: usize,
    max_samples: usize,
    max_preroll_samples: usize,
    candidate_audio: Vec<f32>,
    preroll: VecDeque<f32>,
    voiced_samples: usize,
    active_samples: usize,
    silent_samples: usize,
    candidate_active: bool,
    speech_announced: bool,
    last_input_at: Option<Instant>,
    /// A hard maximum is a technical split, not evidence that speech stopped.
    /// A voiced continuation before this trust expires skips the new-speech
    /// minimum so a short tail is not discarded.
    confirmed_continuation: bool,
    continuation_silent_samples: usize,
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
        let silence_timeout_samples = samples_for_duration(sample_rate, silence_timeout);
        let analysis_frame_samples = (u64::from(sample_rate) * VAD_ANALYSIS_FRAME_MILLIS)
            .div_ceil(1_000)
            .max(1) as usize;

        Self {
            sample_rate,
            rms_threshold,
            silence_timeout_samples,
            analysis_frame_samples,
            analysis_remainder: Vec::with_capacity(analysis_frame_samples),
            min_voiced_samples,
            max_samples,
            max_preroll_samples,
            candidate_audio: Vec::with_capacity(min_voiced_samples),
            preroll: VecDeque::with_capacity(max_preroll_samples),
            voiced_samples: 0,
            active_samples: 0,
            silent_samples: 0,
            candidate_active: false,
            speech_announced: false,
            last_input_at: None,
            confirmed_continuation: false,
            continuation_silent_samples: 0,
        }
    }

    /// Accepts one capture callback and returns every ordered unit transition
    /// caused by it. A callback can cross the hard maximum, so one input may
    /// end the current unit and immediately start one or more confirmed
    /// continuation units without reapplying the new-speech minimum. Capture
    /// callback boundaries do not define energy-analysis boundaries; a short
    /// remainder is retained until one fixed analysis frame is available.
    pub(crate) fn push_samples(&mut self, samples: Vec<f32>, now: Instant) -> Vec<SegmenterUpdate> {
        if samples.is_empty() {
            return meaningful_update(self.tick(now)).into_iter().collect();
        }
        self.last_input_at = Some(now);

        self.analysis_remainder.extend(samples);
        let complete_samples = self.analysis_remainder.len()
            - (self.analysis_remainder.len() % self.analysis_frame_samples);
        if complete_samples == 0 {
            return Vec::new();
        }

        let remainder = self.analysis_remainder.split_off(complete_samples);
        let frames = std::mem::replace(&mut self.analysis_remainder, remainder);
        let mut updates = Vec::new();
        for frame in frames.chunks(self.analysis_frame_samples) {
            self.push_analysis_frame(frame, now, &mut updates);
        }
        updates
    }

    fn push_analysis_frame(
        &mut self,
        samples: &[f32],
        now: Instant,
        updates: &mut Vec<SegmenterUpdate>,
    ) {
        let has_voice = rms(samples) >= self.rms_threshold;
        let mut remaining = samples;

        while !remaining.is_empty() {
            let mut continuation_started = false;
            if has_voice && !self.candidate_active {
                self.candidate_active = true;
                self.candidate_audio.extend(self.preroll.drain(..));
                self.active_samples = self.candidate_audio.len();
                if self.confirmed_continuation {
                    continuation_started = true;
                    self.confirmed_continuation = false;
                    self.continuation_silent_samples = 0;
                }
            }

            if !self.candidate_active {
                if self.confirmed_continuation {
                    self.continuation_silent_samples = self
                        .continuation_silent_samples
                        .saturating_add(remaining.len());
                    if self.continuation_silent_samples >= self.silence_timeout_samples {
                        self.confirmed_continuation = false;
                        self.continuation_silent_samples = 0;
                    }
                }
                self.push_preroll(remaining);
                break;
            }

            let available_samples = self.max_samples.saturating_sub(self.active_samples);
            if available_samples == 0 {
                if self.speech_announced {
                    end_announced_unit(updates);
                }
                self.reset_candidate();
                continue;
            }

            let current_len = remaining.len().min(available_samples);
            let (current, tail) = remaining.split_at(current_len);
            remaining = tail;
            self.push_candidate_samples(current, has_voice, continuation_started, updates);

            let reached_max = self.active_samples >= self.max_samples;
            if reached_max || self.captured_silence_elapsed() {
                if self.speech_announced {
                    end_announced_unit(updates);
                }
                let confirmed_continuation = reached_max && self.speech_announced && has_voice;
                self.reset_candidate();
                self.confirmed_continuation = confirmed_continuation;
                if confirmed_continuation {
                    self.last_input_at = Some(now);
                }
            }
        }

        if self.candidate_active || self.confirmed_continuation {
            self.last_input_at = Some(now);
        }
    }

    pub(crate) fn tick(&mut self, now: Instant) -> SegmenterUpdate {
        if self.confirmed_continuation
            && self.silence_elapsed_without_input(self.continuation_silent_samples, now)
        {
            self.confirmed_continuation = false;
            self.continuation_silent_samples = 0;
            self.last_input_at = None;
            self.analysis_remainder.clear();
        }
        if !self.candidate_active {
            if self.silence_elapsed_without_input(0, now) {
                self.analysis_remainder.clear();
                self.last_input_at = None;
            }
            return SegmenterUpdate::default();
        }
        if !self.silence_elapsed_without_input(self.silent_samples, now) {
            return SegmenterUpdate::default();
        }

        let update = SegmenterUpdate {
            speech_ended: self.speech_announced,
            ..SegmenterUpdate::default()
        };
        self.reset_candidate();
        // The no-input boundary breaks sample continuity. Never join a stale
        // sub-frame tail to audio received after the timeout.
        self.analysis_remainder.clear();
        update
    }

    /// Discards an open tail instead of committing it during Stop or failure.
    pub(crate) fn discard_open_tail(&mut self) -> SegmenterUpdate {
        let update = SegmenterUpdate {
            speech_ended: self.speech_announced,
            ..SegmenterUpdate::default()
        };
        self.reset_candidate();
        self.analysis_remainder.clear();
        update
    }

    pub(crate) fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn captured_silence_elapsed(&self) -> bool {
        self.silent_samples >= self.silence_timeout_samples
    }

    fn silence_elapsed_without_input(&self, silent_samples: usize, now: Instant) -> bool {
        if silent_samples >= self.silence_timeout_samples {
            return true;
        }
        let remaining_samples = self.silence_timeout_samples - silent_samples;
        self.last_input_at.is_some_and(|last_input_at| {
            duration_covers_samples(
                now.saturating_duration_since(last_input_at),
                self.sample_rate,
                remaining_samples,
            )
        })
    }

    fn push_preroll(&mut self, samples: &[f32]) {
        if self.max_preroll_samples == 0 {
            return;
        }
        self.preroll.extend(samples.iter().copied());
        let excess = self.preroll.len().saturating_sub(self.max_preroll_samples);
        if excess > 0 {
            self.preroll.drain(..excess);
        }
    }

    fn push_candidate_samples(
        &mut self,
        samples: &[f32],
        has_voice: bool,
        force_announce: bool,
        updates: &mut Vec<SegmenterUpdate>,
    ) {
        if has_voice {
            self.voiced_samples = self.voiced_samples.saturating_add(samples.len());
            self.silent_samples = 0;
        } else {
            self.silent_samples = self.silent_samples.saturating_add(samples.len());
        }
        self.active_samples = self.active_samples.saturating_add(samples.len());

        if self.speech_announced {
            append_announced_audio(updates, samples);
            return;
        }

        self.candidate_audio.extend_from_slice(samples);
        if !force_announce && self.voiced_samples < self.min_voiced_samples {
            return;
        }

        self.speech_announced = true;
        // Acceptance is the ownership boundary for audio buffered as a candidate.
        updates.push(SegmenterUpdate {
            speech_started: true,
            audio: std::mem::take(&mut self.candidate_audio),
            speech_ended: false,
        });
    }

    fn reset_candidate(&mut self) {
        self.candidate_audio.clear();
        self.preroll.clear();
        self.voiced_samples = 0;
        self.active_samples = 0;
        self.silent_samples = 0;
        self.candidate_active = false;
        self.speech_announced = false;
        self.last_input_at = None;
        self.confirmed_continuation = false;
        self.continuation_silent_samples = 0;
    }
}

fn samples_for_duration(sample_rate: u32, duration: Duration) -> usize {
    let numerator = duration.as_nanos().saturating_mul(u128::from(sample_rate));
    let samples = numerator.saturating_add(999_999_999) / 1_000_000_000;
    samples.clamp(1, usize::MAX as u128) as usize
}

fn duration_covers_samples(duration: Duration, sample_rate: u32, samples: usize) -> bool {
    duration.as_nanos().saturating_mul(u128::from(sample_rate))
        >= (samples as u128).saturating_mul(1_000_000_000)
}

fn meaningful_update(update: SegmenterUpdate) -> Option<SegmenterUpdate> {
    (update != SegmenterUpdate::default()).then_some(update)
}

fn append_announced_audio(updates: &mut Vec<SegmenterUpdate>, samples: &[f32]) {
    if let Some(current) = updates.last_mut()
        && !current.speech_ended
    {
        current.audio.extend_from_slice(samples);
        return;
    }

    // SegmenterUpdate crosses the callback/thread boundary, so the borrowed
    // analysis audio becomes owned once here rather than once per frame.
    updates.push(SegmenterUpdate {
        audio: samples.to_vec(),
        ..SegmenterUpdate::default()
    });
}

fn end_announced_unit(updates: &mut Vec<SegmenterUpdate>) {
    if let Some(current) = updates.last_mut()
        && !current.speech_ended
    {
        current.speech_ended = true;
        return;
    }

    updates.push(SegmenterUpdate {
        speech_ended: true,
        ..SegmenterUpdate::default()
    });
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
