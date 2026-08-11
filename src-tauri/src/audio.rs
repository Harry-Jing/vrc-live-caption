//! Closed facade for microphone capture, scalar level analysis, and local probes.

use crate::error::{AppError, AppResult};

mod capture;
mod level;
mod probe;

pub(crate) const SPEECH_ANALYSIS_FRAME_MILLIS: u64 = 10;
pub(crate) const SPEECH_RMS_THRESHOLD: f32 = 0.012;

pub(crate) use capture::{AudioCapture, AudioInputDevice, list_input_devices, open_input_capture};
pub(crate) use level::{AudioLevelMeter, AudioLevelReading};
pub(crate) use probe::{AudioProbeRequest, AudioProbeResult};

pub(crate) fn speech_gate_level_meter(sample_rate_hz: u32) -> AppResult<AudioLevelMeter> {
    AudioLevelMeter::new(sample_rate_hz, SPEECH_RMS_THRESHOLD)
        .map_err(|error| AppError::audio(error.to_string()))
}

pub(crate) fn probe_audio_input(request: &AudioProbeRequest) -> AppResult<AudioProbeResult> {
    probe::probe_audio_input(request, SPEECH_RMS_THRESHOLD)
}
