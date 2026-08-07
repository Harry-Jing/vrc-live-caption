//! Microphone discovery and capture for the outgoing caption path.
//!
//! Device ids are CPAL `DeviceId` strings, not display names. Persisting
//! names is fragile because duplicate names and reconnects are common on Windows.
//! Captured samples are converted to mono `f32` frames before reaching runtime
//! code; the frontend never sees raw audio.

use crate::config::AudioConfig;
use crate::error::{AppError, AppResult};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    DeviceId, Error as CpalError, ErrorKind, FromSample, I24, Sample, SampleFormat, SizedSample,
    Stream, StreamConfig, U24,
};
use serde::Serialize;
use std::str::FromStr;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::time::Duration;

// CPAL treats `None` as an unbounded backend initialization wait. A finite
// request keeps Start/Stop from deliberately opting into an infinite wait;
// some platform backends may still be unable to honor it.
const STREAM_INIT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AudioInputDevice {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) is_default: bool,
}

pub(crate) struct AudioCapture {
    pub(crate) receiver: AudioCaptureReceiver,
    pub(crate) sample_rate: u32,
    pub(crate) stream: Stream,
}

pub(crate) struct AudioCaptureReceiver {
    samples: Receiver<Vec<f32>>,
    dropped_frames: Receiver<()>,
    fatal_errors: Receiver<CpalError>,
    notifications: Receiver<CpalError>,
}

type InputStreamBuilder = fn(
    &cpal::Device,
    StreamConfig,
    usize,
    SyncSender<Vec<f32>>,
    SyncSender<()>,
    SyncSender<CpalError>,
    SyncSender<CpalError>,
) -> AppResult<Stream>;

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
    let (sample_sender, sample_receiver) = sync_channel(16);
    // A full sample queue must never turn into a silently corrupted
    // transcript. The callback latches one gap notification without blocking;
    // runtime treats it as terminal and asks the user to retry.
    let (dropped_frame_sender, dropped_frame_receiver) = sync_channel(1);
    // Fatal stream errors use an independent one-slot latch. Sharing the
    // bounded sample queue would allow a full audio backlog to drop the only
    // signal that tells the runtime to leave Running.
    let (fatal_error_sender, fatal_error_receiver) = sync_channel(1);
    // Recoverable CPAL notifications are best-effort and cannot occupy the
    // fatal latch. They are logged by the runtime thread, never this callback.
    let (notification_sender, notification_receiver) = sync_channel(1);
    let stream_builder = input_stream_builder(sample_format)?;
    let stream = stream_builder(
        &device,
        stream_config,
        channels,
        sample_sender,
        dropped_frame_sender,
        fatal_error_sender,
        notification_sender,
    )?;

    stream.play().map_err(|error| {
        AppError::audio(format!(
            "Failed to start microphone capture on {device_name}: {error}"
        ))
    })?;

    Ok(AudioCapture {
        receiver: AudioCaptureReceiver {
            samples: sample_receiver,
            dropped_frames: dropped_frame_receiver,
            fatal_errors: fatal_error_receiver,
            notifications: notification_receiver,
        },
        sample_rate,
        stream,
    })
}

fn input_stream_builder(sample_format: SampleFormat) -> AppResult<InputStreamBuilder> {
    match sample_format {
        SampleFormat::F32 => Ok(build_input_stream::<f32>),
        SampleFormat::F64 => Ok(build_input_stream::<f64>),
        SampleFormat::I8 => Ok(build_input_stream::<i8>),
        SampleFormat::I16 => Ok(build_input_stream::<i16>),
        SampleFormat::I24 => Ok(build_input_stream::<I24>),
        SampleFormat::I32 => Ok(build_input_stream::<i32>),
        SampleFormat::I64 => Ok(build_input_stream::<i64>),
        SampleFormat::U8 => Ok(build_input_stream::<u8>),
        SampleFormat::U16 => Ok(build_input_stream::<u16>),
        SampleFormat::U24 => Ok(build_input_stream::<U24>),
        SampleFormat::U32 => Ok(build_input_stream::<u32>),
        SampleFormat::U64 => Ok(build_input_stream::<u64>),
        SampleFormat::DsdU8 | SampleFormat::DsdU16 | SampleFormat::DsdU32 => {
            Err(AppError::audio(format!(
                "DSD microphone sample format is not PCM and cannot be captured: {sample_format:?}"
            )))
        }
        _ => Err(AppError::audio(format!(
            "Unsupported microphone sample format: {sample_format:?}"
        ))),
    }
}

pub(crate) fn receive_audio(
    receiver: &AudioCaptureReceiver,
    timeout: Duration,
) -> AppResult<Option<Vec<f32>>> {
    check_stream_failure(&receiver.fatal_errors)?;
    check_capture_gap(&receiver.dropped_frames)?;
    log_stream_notifications(&receiver.notifications);

    let result = match receiver.samples.recv_timeout(timeout) {
        Ok(samples) => Ok(Some(samples)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(None),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(AppError::audio(
            "Microphone capture stopped unexpectedly because the input stream disconnected.",
        )),
    };

    // An error may arrive while recv_timeout is waiting. Check again before
    // returning even a buffered sample so a permanent failure cannot be
    // starved by queued audio.
    check_stream_failure(&receiver.fatal_errors)?;
    check_capture_gap(&receiver.dropped_frames)?;
    log_stream_notifications(&receiver.notifications);

    result
}

fn check_capture_gap(receiver: &Receiver<()>) -> AppResult<()> {
    match receiver.try_recv() {
        Ok(()) => Err(AppError::audio(
            "Microphone audio frames were dropped because recognition could not keep up. The runtime stopped instead of transcribing incomplete audio.",
        )),
        Err(TryRecvError::Empty) => Ok(()),
        Err(TryRecvError::Disconnected) => Err(AppError::audio(
            "Microphone capture stopped unexpectedly because its backpressure monitor disconnected.",
        )),
    }
}

fn check_stream_failure(receiver: &Receiver<CpalError>) -> AppResult<()> {
    match receiver.try_recv() {
        Ok(error) => Err(AppError::audio(format!(
            "Microphone input stream stopped unexpectedly: {error}"
        ))),
        Err(TryRecvError::Empty) => Ok(()),
        Err(TryRecvError::Disconnected) => Err(AppError::audio(
            "Microphone input stream stopped unexpectedly because its error monitor disconnected.",
        )),
    }
}

fn log_stream_notifications(receiver: &Receiver<CpalError>) {
    while let Ok(error) = receiver.try_recv() {
        tracing::warn!(
            error_kind = ?error.kind(),
            error_message = %error,
            terminal = false,
            "microphone input stream notification"
        );
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
    config: StreamConfig,
    channels: usize,
    sample_sender: SyncSender<Vec<f32>>,
    dropped_frame_sender: SyncSender<()>,
    fatal_error_sender: SyncSender<CpalError>,
    notification_sender: SyncSender<CpalError>,
) -> AppResult<Stream>
where
    T: Sample + SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                write_mono_samples(data, channels, &sample_sender, &dropped_frame_sender);
            },
            move |error| {
                route_stream_error(error, &fatal_error_sender, &notification_sender);
            },
            Some(STREAM_INIT_TIMEOUT),
        )
        .map_err(|error| {
            AppError::audio(format!("Failed to build microphone input stream: {error}"))
        })
}

fn route_stream_error(
    error: CpalError,
    fatal_error_sender: &SyncSender<CpalError>,
    notification_sender: &SyncSender<CpalError>,
) {
    let recoverable = matches!(
        error.kind(),
        ErrorKind::DeviceChanged | ErrorKind::RealtimeDenied | ErrorKind::Xrun
    );
    if recoverable {
        // Repeated warnings may be coalesced; they must never block the audio
        // backend or consume the fatal-error slot.
        let _ = notification_sender.try_send(error);
    } else {
        // The first fatal error is enough to end this generation. Full means a
        // fatal signal is already latched; Disconnected means cleanup won.
        let _ = fatal_error_sender.try_send(error);
    }
}

fn write_mono_samples<T>(
    input: &[T],
    channels: usize,
    sender: &SyncSender<Vec<f32>>,
    dropped_frame_sender: &SyncSender<()>,
) where
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

    if !samples.is_empty() && matches!(sender.try_send(samples), Err(TrySendError::Full(_))) {
        let _ = dropped_frame_sender.try_send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_failure_wins_over_buffered_audio_instead_of_hanging() -> AppResult<()> {
        let (sample_sender, sample_receiver) = sync_channel(1);
        let (_dropped_frame_sender, dropped_frame_receiver) = sync_channel(1);
        let (fatal_error_sender, fatal_error_receiver) = sync_channel(1);
        let (notification_sender, notification_receiver) = sync_channel(1);
        sample_sender
            .send(vec![0.25])
            .map_err(|_| AppError::audio("Failed to buffer the test audio frame."))?;
        route_stream_error(
            CpalError::with_message(
                ErrorKind::DeviceNotAvailable,
                "The microphone was disconnected.",
            ),
            &fatal_error_sender,
            &notification_sender,
        );
        let receiver = AudioCaptureReceiver {
            samples: sample_receiver,
            dropped_frames: dropped_frame_receiver,
            fatal_errors: fatal_error_receiver,
            notifications: notification_receiver,
        };

        let error = receive_audio(&receiver, Duration::ZERO)
            .err()
            .ok_or_else(|| AppError::audio("Stream failure was hidden by buffered audio."))?;

        assert_eq!(error.code(), "audio.failed");
        assert!(error.to_string().contains("microphone was disconnected"));
        Ok(())
    }

    #[test]
    fn recoverable_stream_notifications_do_not_stop_audio_capture() -> AppResult<()> {
        for error_kind in [
            ErrorKind::DeviceChanged,
            ErrorKind::RealtimeDenied,
            ErrorKind::Xrun,
        ] {
            let (sample_sender, sample_receiver) = sync_channel(1);
            let (_dropped_frame_sender, dropped_frame_receiver) = sync_channel(1);
            let (fatal_error_sender, fatal_error_receiver) = sync_channel(1);
            let (notification_sender, notification_receiver) = sync_channel(1);
            sample_sender
                .send(vec![0.5])
                .map_err(|_| AppError::audio("Failed to buffer the test audio frame."))?;
            route_stream_error(
                CpalError::new(error_kind),
                &fatal_error_sender,
                &notification_sender,
            );
            let receiver = AudioCaptureReceiver {
                samples: sample_receiver,
                dropped_frames: dropped_frame_receiver,
                fatal_errors: fatal_error_receiver,
                notifications: notification_receiver,
            };

            assert_eq!(receive_audio(&receiver, Duration::ZERO)?, Some(vec![0.5]));
        }

        Ok(())
    }

    #[test]
    fn full_sample_queue_latches_a_visible_capture_gap() -> AppResult<()> {
        let (sample_sender, sample_receiver) = sync_channel(1);
        let (dropped_frame_sender, dropped_frame_receiver) = sync_channel(1);
        let (_fatal_error_sender, fatal_error_receiver) = sync_channel(1);
        let (_notification_sender, notification_receiver) = sync_channel(1);
        write_mono_samples(&[0.25_f32], 1, &sample_sender, &dropped_frame_sender);
        write_mono_samples(&[0.5_f32], 1, &sample_sender, &dropped_frame_sender);
        let receiver = AudioCaptureReceiver {
            samples: sample_receiver,
            dropped_frames: dropped_frame_receiver,
            fatal_errors: fatal_error_receiver,
            notifications: notification_receiver,
        };

        let error = receive_audio(&receiver, Duration::ZERO)
            .err()
            .ok_or_else(|| AppError::audio("Dropped audio was not reported."))?;

        assert!(error.to_string().contains("frames were dropped"));
        Ok(())
    }

    #[test]
    fn every_pcm_sample_format_has_an_input_stream_builder() {
        let pcm_formats = [
            SampleFormat::I8,
            SampleFormat::I16,
            SampleFormat::I24,
            SampleFormat::I32,
            SampleFormat::I64,
            SampleFormat::U8,
            SampleFormat::U16,
            SampleFormat::U24,
            SampleFormat::U32,
            SampleFormat::U64,
            SampleFormat::F32,
            SampleFormat::F64,
        ];

        for sample_format in pcm_formats {
            assert!(
                input_stream_builder(sample_format).is_ok(),
                "missing input stream builder for {sample_format:?}"
            );
        }
    }

    #[test]
    fn dsd_sample_formats_are_rejected_as_non_pcm() {
        let dsd_formats = [
            SampleFormat::DsdU8,
            SampleFormat::DsdU16,
            SampleFormat::DsdU32,
        ];

        for sample_format in dsd_formats {
            let error_message = input_stream_builder(sample_format)
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default();

            assert!(
                error_message.contains("DSD microphone sample format is not PCM"),
                "DSD format was not rejected explicitly: {sample_format:?}"
            );
        }
    }
}
