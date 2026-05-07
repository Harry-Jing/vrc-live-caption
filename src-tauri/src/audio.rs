//! Microphone discovery and capture for the outgoing caption path.
//!
//! Device ids are CPAL 0.17 `DeviceId` strings, not display names. Persisting
//! names is fragile because duplicate names and reconnects are common on Windows.
//! Captured samples are converted to mono `f32` frames before reaching runtime
//! code; the frontend never sees raw audio.

use crate::config::AudioConfig;
use crate::error::{AppError, AppResult};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{DeviceId, FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig};
use serde::Serialize;
use std::str::FromStr;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::Duration;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AudioInputDevice {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) is_default: bool,
}

pub(crate) struct AudioCapture {
    pub(crate) receiver: Receiver<Vec<f32>>,
    pub(crate) sample_rate: u32,
    pub(crate) stream: Stream,
}

pub(crate) fn list_input_devices() -> AppResult<Vec<AudioInputDevice>> {
    let host = cpal::default_host();
    let default_id = host
        .default_input_device()
        .and_then(|device| device.id().ok())
        .map(|id| id.to_string());
    let devices = host
        .input_devices()
        .map_err(|error| AppError::audio(format!("Failed to list input devices: {error}")))?;
    let mut input_devices = Vec::new();

    for device in devices {
        let description = match device.description() {
            Ok(description) => description,
            Err(error) => {
                tracing::warn!(error_message = %error, "skipping undescribed input device");
                continue;
            }
        };
        let id = match device.id() {
            Ok(id) => id.to_string(),
            Err(error) => {
                tracing::warn!(
                    error_message = %error,
                    device_name = description.name(),
                    "skipping input device without stable id"
                );
                continue;
            }
        };

        input_devices.push(AudioInputDevice {
            is_default: default_id.as_deref() == Some(id.as_str()),
            id,
            name: description.name().to_string(),
        });
    }

    Ok(input_devices)
}

pub(crate) fn open_input_capture(config: &AudioConfig) -> AppResult<AudioCapture> {
    let host = cpal::default_host();
    let device = select_input_device(&host, config.input_device_id.as_deref())?;
    let device_name = device
        .description()
        .map(|description| description.name().to_string())
        .unwrap_or_else(|_| "Selected input device".to_string());
    let supported_config = device.default_input_config().map_err(|error| {
        AppError::audio(format!(
            "Failed to read default input config for {device_name}: {error}"
        ))
    })?;
    let sample_rate = supported_config.sample_rate();
    let channels = usize::from(supported_config.channels());
    let sample_format = supported_config.sample_format();
    let stream_config: StreamConfig = supported_config.into();
    let (sender, receiver) = sync_channel(16);
    let stream = match sample_format {
        SampleFormat::F32 => build_input_stream::<f32>(&device, &stream_config, channels, sender),
        SampleFormat::I16 => build_input_stream::<i16>(&device, &stream_config, channels, sender),
        SampleFormat::U16 => build_input_stream::<u16>(&device, &stream_config, channels, sender),
        _ => Err(AppError::audio(format!(
            "Unsupported microphone sample format: {sample_format:?}"
        ))),
    }?;

    stream.play().map_err(|error| {
        AppError::audio(format!(
            "Failed to start microphone capture on {device_name}: {error}"
        ))
    })?;

    Ok(AudioCapture {
        receiver,
        sample_rate,
        stream,
    })
}

pub(crate) fn receive_audio(
    receiver: &Receiver<Vec<f32>>,
    timeout: Duration,
) -> AppResult<Option<Vec<f32>>> {
    match receiver.recv_timeout(timeout) {
        Ok(samples) => Ok(Some(samples)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(None),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(AppError::audio(
            "Microphone capture stopped unexpectedly because the input stream disconnected.",
        )),
    }
}

fn select_input_device(
    host: &cpal::Host,
    input_device_id: Option<&str>,
) -> AppResult<cpal::Device> {
    if let Some(input_device_id) = input_device_id {
        let device_id = DeviceId::from_str(input_device_id).map_err(|error| {
            AppError::audio(format!(
                "Selected microphone id is not valid: {input_device_id}: {error}"
            ))
        })?;

        return host.device_by_id(&device_id).ok_or_else(|| {
            AppError::audio(format!(
                "Selected microphone was not found: {input_device_id}"
            ))
        });
    }

    host.default_input_device()
        .ok_or_else(|| AppError::audio("No default microphone input device was found."))
}

fn build_input_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    channels: usize,
    sender: SyncSender<Vec<f32>>,
) -> AppResult<Stream>
where
    T: Sample + SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    device
        .build_input_stream(
            config,
            move |data: &[T], _| write_mono_samples(data, channels, &sender),
            move |error| {
                tracing::warn!(error_message = %error, "microphone input stream error");
            },
            None,
        )
        .map_err(|error| {
            AppError::audio(format!("Failed to build microphone input stream: {error}"))
        })
}

fn write_mono_samples<T>(input: &[T], channels: usize, sender: &SyncSender<Vec<f32>>)
where
    T: Sample,
    f32: FromSample<T>,
{
    if channels == 0 {
        return;
    }

    let mut samples = Vec::with_capacity(input.len() / channels);

    for frame in input.chunks(channels) {
        let mut sum = 0.0;

        for sample in frame {
            sum += sample.to_sample::<f32>();
        }

        samples.push(sum / channels as f32);
    }

    if !samples.is_empty() {
        let _ = sender.try_send(samples);
    }
}
