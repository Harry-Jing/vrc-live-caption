//! Fixed-window microphone level aggregation.
//!
//! The meter accepts mono PCM samples and emits only scalar statistics. Audio
//! never crosses this module's output boundary.

use super::SPEECH_ANALYSIS_FRAME_MILLIS;
use std::error::Error;
use std::fmt;

const LEVEL_WINDOW_MILLIS: u64 = 100;
pub(super) const TELEMETRY_DBFS_FLOOR: f32 = -120.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AudioLevelReading {
    pub(crate) rms_dbfs: f32,
    pub(crate) peak_dbfs: f32,
    pub(crate) clipping: bool,
    pub(crate) vad_gate_open: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AudioLevelConfigError {
    ZeroSampleRate,
    InvalidGateThreshold,
}

impl fmt::Display for AudioLevelConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::ZeroSampleRate => "Audio level sample rate must be greater than zero.",
            Self::InvalidGateThreshold => {
                "Audio level gate threshold must be finite, greater than zero, and at most one."
            }
        };
        formatter.write_str(message)
    }
}

impl Error for AudioLevelConfigError {}

pub(crate) struct AudioLevelMeter {
    window_samples: usize,
    gate_frame_samples: usize,
    gate_rms_threshold: f32,
    collected_samples: usize,
    sum_of_squares: f64,
    peak: f32,
    clipping: bool,
    gate_collected_samples: usize,
    gate_sum_of_squares: f64,
    vad_gate_open: bool,
}

impl AudioLevelMeter {
    pub(super) fn new(
        sample_rate_hz: u32,
        gate_rms_threshold: f32,
    ) -> Result<Self, AudioLevelConfigError> {
        if sample_rate_hz == 0 {
            return Err(AudioLevelConfigError::ZeroSampleRate);
        }
        if !gate_rms_threshold.is_finite()
            || !(0.0 < gate_rms_threshold && gate_rms_threshold <= 1.0)
        {
            return Err(AudioLevelConfigError::InvalidGateThreshold);
        }
        let window_samples = (u64::from(sample_rate_hz) * LEVEL_WINDOW_MILLIS)
            .div_ceil(1_000)
            .max(1) as usize;
        let gate_frame_samples = (u64::from(sample_rate_hz) * SPEECH_ANALYSIS_FRAME_MILLIS)
            .div_ceil(1_000)
            .max(1) as usize;
        Ok(Self {
            window_samples,
            gate_frame_samples,
            gate_rms_threshold,
            collected_samples: 0,
            sum_of_squares: 0.0,
            peak: 0.0,
            clipping: false,
            gate_collected_samples: 0,
            gate_sum_of_squares: 0.0,
            vad_gate_open: false,
        })
    }

    pub(crate) fn push_samples(&mut self, samples: &[f32]) -> Vec<AudioLevelReading> {
        let mut readings = Vec::new();
        for sample in samples {
            let magnitude = if sample.is_finite() {
                sample.abs()
            } else {
                // Invalid capture values must not poison UI-facing scalar
                // telemetry. Treat them as full-scale so the window remains
                // finite and visibly reports clipping.
                1.0
            };
            self.collected_samples = self.collected_samples.saturating_add(1);
            self.sum_of_squares += f64::from(magnitude) * f64::from(magnitude);
            self.peak = self.peak.max(magnitude);
            self.clipping |= magnitude >= 1.0;
            self.gate_collected_samples = self.gate_collected_samples.saturating_add(1);
            self.gate_sum_of_squares += f64::from(magnitude) * f64::from(magnitude);

            if self.gate_collected_samples == self.gate_frame_samples {
                let gate_rms =
                    (self.gate_sum_of_squares / self.gate_collected_samples as f64).sqrt() as f32;
                self.vad_gate_open |= gate_rms >= self.gate_rms_threshold;
                self.gate_collected_samples = 0;
                self.gate_sum_of_squares = 0.0;
            }

            if self.collected_samples == self.window_samples {
                readings.push(self.finish_window());
            }
        }
        readings
    }

    fn finish_window(&mut self) -> AudioLevelReading {
        let rms = (self.sum_of_squares / self.collected_samples as f64).sqrt() as f32;
        let reading = AudioLevelReading {
            rms_dbfs: amplitude_to_dbfs(rms),
            peak_dbfs: amplitude_to_dbfs(self.peak),
            clipping: self.clipping,
            vad_gate_open: self.vad_gate_open,
        };
        self.collected_samples = 0;
        self.sum_of_squares = 0.0;
        self.peak = 0.0;
        self.clipping = false;
        self.vad_gate_open = false;
        reading
    }
}

fn amplitude_to_dbfs(amplitude: f32) -> f32 {
    if amplitude <= 0.0 {
        return TELEMETRY_DBFS_FLOOR;
    }
    (20.0 * amplitude.log10()).max(TELEMETRY_DBFS_FLOOR)
}

#[cfg(test)]
#[path = "level_tests.rs"]
mod tests;
