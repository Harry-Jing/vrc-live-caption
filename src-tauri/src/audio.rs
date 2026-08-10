//! Closed facade for microphone capture, scalar level analysis, and local probes.

mod capture;
mod level;
mod probe;

pub(crate) use capture::{
    AudioCapture, AudioInputDevice, list_input_devices, open_input_capture, receive_audio,
};
pub(crate) use level::{AudioLevelMeter, SPEECH_RMS_THRESHOLD, VAD_ANALYSIS_FRAME_MILLIS};
pub(crate) use probe::{AudioProbeRequest, AudioProbeResult, probe_audio_input};
