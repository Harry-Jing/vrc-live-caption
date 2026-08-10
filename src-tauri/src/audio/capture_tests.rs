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
