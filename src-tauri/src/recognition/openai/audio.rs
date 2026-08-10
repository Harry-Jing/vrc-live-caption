//! Stateful mono floating-point audio conversion for Realtime transcription.

use crate::error::{AppError, AppResult};

pub(crate) const REALTIME_PCM_SAMPLE_RATE_HZ: u32 = 24_000;

/// Converts a fixed-rate mono capture stream into signed 16-bit little-endian
/// PCM at exactly 24 kHz. Linear interpolation is intentionally kept behind
/// this small boundary so it can later be replaced without leaking resampler
/// details into the provider adapter.
pub(crate) struct RealtimePcm16Encoder {
    input_sample_rate_hz: Option<u32>,
    pending_samples: Vec<f32>,
    next_position: f64,
}

impl RealtimePcm16Encoder {
    pub(crate) fn new() -> Self {
        Self {
            input_sample_rate_hz: None,
            pending_samples: Vec::new(),
            next_position: 0.0,
        }
    }

    pub(crate) fn append(&mut self, sample_rate_hz: u32, samples: &[f32]) -> AppResult<Vec<u8>> {
        self.validate_input(sample_rate_hz, samples)?;
        if samples.is_empty() {
            return Ok(Vec::new());
        }

        if sample_rate_hz == REALTIME_PCM_SAMPLE_RATE_HZ {
            return Ok(encode_samples(samples));
        }

        self.pending_samples.extend_from_slice(samples);
        let step = f64::from(sample_rate_hz) / f64::from(REALTIME_PCM_SAMPLE_RATE_HZ);
        let mut output = Vec::new();
        while self.next_position + 1.0 < self.pending_samples.len() as f64 {
            let sample = self.interpolated_sample(self.next_position);
            push_pcm16(&mut output, sample);
            self.next_position += step;
        }
        self.discard_consumed_prefix();
        Ok(output)
    }

    pub(crate) fn finish_unit(&mut self) -> Vec<u8> {
        let Some(input_sample_rate_hz) = self.input_sample_rate_hz else {
            return Vec::new();
        };
        if input_sample_rate_hz == REALTIME_PCM_SAMPLE_RATE_HZ {
            return Vec::new();
        }

        let step = f64::from(input_sample_rate_hz) / f64::from(REALTIME_PCM_SAMPLE_RATE_HZ);
        let mut output = Vec::new();
        while self.next_position < self.pending_samples.len() as f64 {
            let sample = self.interpolated_sample(self.next_position);
            push_pcm16(&mut output, sample);
            self.next_position += step;
        }
        self.reset_unit();
        output
    }

    pub(crate) fn reset_unit(&mut self) {
        self.pending_samples.clear();
        self.next_position = 0.0;
    }

    fn validate_input(&mut self, sample_rate_hz: u32, samples: &[f32]) -> AppResult<()> {
        if sample_rate_hz == 0 {
            return Err(AppError::audio(
                "Recognition audio sample rate must be greater than zero.",
            ));
        }
        if samples.iter().any(|sample| !sample.is_finite()) {
            return Err(AppError::audio(
                "Recognition audio contains a non-finite sample.",
            ));
        }

        match self.input_sample_rate_hz {
            Some(current) if current != sample_rate_hz => Err(AppError::audio(format!(
                "Recognition audio sample rate changed from {current} Hz to {sample_rate_hz} Hz during one session."
            ))),
            Some(_) => Ok(()),
            None => {
                self.input_sample_rate_hz = Some(sample_rate_hz);
                Ok(())
            }
        }
    }

    fn interpolated_sample(&self, position: f64) -> f32 {
        let lower_index = position.floor() as usize;
        let upper_index = lower_index
            .saturating_add(1)
            .min(self.pending_samples.len().saturating_sub(1));
        let fraction = (position - lower_index as f64) as f32;
        let lower = self.pending_samples[lower_index];
        let upper = self.pending_samples[upper_index];
        lower + (upper - lower) * fraction
    }

    fn discard_consumed_prefix(&mut self) {
        let consumed = self.next_position.floor() as usize;
        let discard = consumed.min(self.pending_samples.len().saturating_sub(1));
        if discard > 0 {
            self.pending_samples.drain(..discard);
            self.next_position -= discard as f64;
        }
    }
}

fn encode_samples(samples: &[f32]) -> Vec<u8> {
    let mut output = Vec::with_capacity(samples.len().saturating_mul(2));
    for sample in samples {
        push_pcm16(&mut output, *sample);
    }
    output
}

fn push_pcm16(output: &mut Vec<u8>, sample: f32) {
    let encoded = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
    output.extend_from_slice(&encoded.to_le_bytes());
}

#[cfg(test)]
#[path = "audio_tests.rs"]
mod tests;
