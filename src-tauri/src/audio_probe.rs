//! Short, local-only microphone probe that returns scalar level statistics.

use crate::audio::{open_input_capture, receive_audio};
use crate::audio_level::{AudioLevelMeter, DBFS_FLOOR};
use crate::config::AudioConfig;
use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

const RECEIVE_TIMEOUT: Duration = Duration::from_millis(100);
const MIN_PROBE_DURATION_MILLIS: u64 = 500;
const MAX_PROBE_DURATION_MILLIS: u64 = 5_000;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AudioProbeRequest {
    pub(crate) input_device_id: Option<String>,
    pub(crate) duration_ms: u64,
}

impl AudioProbeRequest {
    fn duration(&self) -> AppResult<Duration> {
        if !(MIN_PROBE_DURATION_MILLIS..=MAX_PROBE_DURATION_MILLIS).contains(&self.duration_ms) {
            return Err(AppError::audio(format!(
                "Microphone probe duration must be between {MIN_PROBE_DURATION_MILLIS} and {MAX_PROBE_DURATION_MILLIS} milliseconds."
            )));
        }
        Ok(Duration::from_millis(self.duration_ms))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AudioProbeResult {
    /// Actual sample rate selected by the input device.
    pub(crate) sample_rate: u32,
    /// Fixed observation interval requested by this probe.
    pub(crate) duration_ms: u64,
    /// Loudest RMS among complete level-meter windows.
    pub(crate) rms_dbfs: f32,
    /// Loudest peak among complete level-meter windows.
    pub(crate) peak_dbfs: f32,
    pub(crate) clipping: bool,
    pub(crate) gate_open: bool,
}

/// Opens the configured microphone for one short local observation and closes
/// it before returning. Only scalar statistics cross this interface; captured
/// PCM is consumed in memory and never returned or persisted.
pub(crate) fn probe_audio_input(
    request: &AudioProbeRequest,
    gate_rms_threshold: f32,
) -> AppResult<AudioProbeResult> {
    let duration = request.duration()?;
    let config = AudioConfig {
        input_device_id: request.input_device_id.clone(),
    };
    let capture = open_input_capture(&config)?;
    collect_probe_with(
        capture.sample_rate,
        gate_rms_threshold,
        duration,
        |timeout| receive_audio(&capture.receiver, timeout),
        Instant::now,
    )
}

fn collect_probe_with<R, N>(
    sample_rate: u32,
    gate_rms_threshold: f32,
    duration: Duration,
    mut receive: R,
    mut now: N,
) -> AppResult<AudioProbeResult>
where
    R: FnMut(Duration) -> AppResult<Option<Vec<f32>>>,
    N: FnMut() -> Instant,
{
    let mut meter = AudioLevelMeter::new(sample_rate, gate_rms_threshold).map_err(|error| {
        AppError::audio(format!("Failed to configure microphone probe: {error}"))
    })?;
    let started_at = now();
    let deadline = started_at.checked_add(duration).ok_or_else(|| {
        AppError::audio("Microphone probe duration exceeded the monotonic clock range.")
    })?;
    let mut rms_dbfs = DBFS_FLOOR;
    let mut peak_dbfs = DBFS_FLOOR;
    let mut clipping = false;
    let mut gate_open = false;

    loop {
        let current = now();
        if current >= deadline {
            break;
        }
        let timeout = deadline
            .saturating_duration_since(current)
            .min(RECEIVE_TIMEOUT);
        if let Some(samples) = receive(timeout)? {
            for reading in meter.push_samples(&samples) {
                rms_dbfs = rms_dbfs.max(reading.rms_dbfs);
                peak_dbfs = peak_dbfs.max(reading.peak_dbfs);
                clipping |= reading.clipping;
                gate_open |= reading.vad_gate_open;
            }
        }
    }

    Ok(AudioProbeResult {
        sample_rate,
        duration_ms: duration.as_millis().min(u128::from(u64::MAX)) as u64,
        rms_dbfs,
        peak_dbfs,
        clipping,
        gate_open,
    })
}

#[cfg(test)]
#[path = "audio_probe_tests.rs"]
mod tests;
