use super::*;
use std::collections::VecDeque;

#[test]
fn request_duration_is_bounded_for_a_short_local_probe() -> AppResult<()> {
    for duration_ms in [500, 2_000, 5_000] {
        let request = AudioProbeRequest {
            input_device_id: None,
            duration_ms,
        };
        assert_eq!(request.duration()?, Duration::from_millis(duration_ms));
    }

    for duration_ms in [0, 499, 5_001, u64::MAX] {
        let request = AudioProbeRequest {
            input_device_id: None,
            duration_ms,
        };
        assert_eq!(
            request.duration().err().map(|error| error.code()),
            Some("audio.failed")
        );
    }
    Ok(())
}

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 0.001,
        "expected {actual} to be close to {expected}"
    );
}

fn clock_at(started_at: Instant, offsets: &[Duration]) -> impl FnMut() -> Instant {
    let mut times = offsets
        .iter()
        .map(|offset| started_at + *offset)
        .collect::<VecDeque<_>>();
    let last = times.back().copied().unwrap_or(started_at);
    move || times.pop_front().unwrap_or(last)
}

#[test]
fn probe_keeps_the_loudest_complete_window_instead_of_averaging_the_whole_run() -> AppResult<()> {
    let started_at = Instant::now();
    let mut frames = VecDeque::from([Some({
        let mut samples = vec![0.01; 100];
        samples.extend(vec![0.5; 100]);
        samples
    })]);

    let result = collect_probe_with(
        1_000,
        0.1,
        Duration::from_secs(2),
        |_| Ok(frames.pop_front().flatten()),
        clock_at(
            started_at,
            &[Duration::ZERO, Duration::ZERO, Duration::from_secs(2)],
        ),
    )?;

    assert_eq!(result.sample_rate, 1_000);
    assert_eq!(result.duration_ms, 2_000);
    assert_close(result.rms_dbfs, -6.0206);
    assert_close(result.peak_dbfs, -6.0206);
    assert!(!result.clipping);
    assert!(result.gate_open);
    Ok(())
}

#[test]
fn silence_and_an_incomplete_window_use_a_finite_floor() -> AppResult<()> {
    let started_at = Instant::now();
    let mut silent_frames = VecDeque::from([Some(vec![0.0; 100])]);
    let silence = collect_probe_with(
        1_000,
        0.012,
        Duration::from_secs(2),
        |_| Ok(silent_frames.pop_front().flatten()),
        clock_at(
            started_at,
            &[Duration::ZERO, Duration::ZERO, Duration::from_secs(2)],
        ),
    )?;

    let mut partial_frames = VecDeque::from([Some(vec![0.5; 99])]);
    let partial = collect_probe_with(
        1_000,
        0.012,
        Duration::from_secs(2),
        |_| Ok(partial_frames.pop_front().flatten()),
        clock_at(
            started_at,
            &[Duration::ZERO, Duration::ZERO, Duration::from_secs(2)],
        ),
    )?;

    for result in [silence, partial] {
        assert_eq!(result.rms_dbfs, TELEMETRY_DBFS_FLOOR);
        assert_eq!(result.peak_dbfs, TELEMETRY_DBFS_FLOOR);
        assert!(result.rms_dbfs.is_finite());
        assert!(result.peak_dbfs.is_finite());
        assert!(!result.clipping);
        assert!(!result.gate_open);
    }
    Ok(())
}

#[test]
fn clipping_and_peak_are_preserved_across_complete_windows() -> AppResult<()> {
    let started_at = Instant::now();
    let mut samples = vec![0.8; 100];
    samples.extend(vec![0.1; 100]);
    samples[137] = -1.0;
    let mut frames = VecDeque::from([Some(samples)]);

    let result = collect_probe_with(
        1_000,
        0.012,
        Duration::from_secs(2),
        |_| Ok(frames.pop_front().flatten()),
        clock_at(
            started_at,
            &[Duration::ZERO, Duration::ZERO, Duration::from_secs(2)],
        ),
    )?;

    assert_close(result.rms_dbfs, -1.9382);
    assert_eq!(result.peak_dbfs, 0.0);
    assert!(result.clipping);
    assert!(result.gate_open);
    Ok(())
}

#[test]
fn receive_timeout_is_non_terminal_and_the_probe_still_observes_later_audio() -> AppResult<()> {
    let started_at = Instant::now();
    let mut frames = VecDeque::from([None, Some(vec![0.5; 100])]);
    let mut requested_timeouts = Vec::new();

    let result = collect_probe_with(
        1_000,
        0.1,
        Duration::from_secs(2),
        |timeout| {
            requested_timeouts.push(timeout);
            Ok(frames.pop_front().flatten())
        },
        clock_at(
            started_at,
            &[
                Duration::ZERO,
                Duration::ZERO,
                Duration::from_millis(100),
                Duration::from_secs(2),
            ],
        ),
    )?;

    assert_eq!(requested_timeouts, vec![RECEIVE_TIMEOUT, RECEIVE_TIMEOUT]);
    assert_close(result.rms_dbfs, -6.0206);
    assert!(result.gate_open);
    Ok(())
}

#[test]
fn a_probe_with_only_receive_timeouts_returns_finite_floor_values() -> AppResult<()> {
    let started_at = Instant::now();

    let result = collect_probe_with(
        48_000,
        0.012,
        Duration::from_secs(2),
        |_| Ok(None),
        clock_at(
            started_at,
            &[Duration::ZERO, Duration::ZERO, Duration::from_secs(2)],
        ),
    )?;

    assert_eq!(result.rms_dbfs, TELEMETRY_DBFS_FLOOR);
    assert_eq!(result.peak_dbfs, TELEMETRY_DBFS_FLOOR);
    assert!(result.rms_dbfs.is_finite());
    assert!(result.peak_dbfs.is_finite());
    assert!(!result.clipping);
    assert!(!result.gate_open);
    Ok(())
}

#[test]
fn capture_error_is_returned_instead_of_a_partial_probe_result() {
    let started_at = Instant::now();
    let mut first_receive = true;
    let error = collect_probe_with(
        1_000,
        0.012,
        Duration::from_secs(2),
        |_| {
            if std::mem::take(&mut first_receive) {
                Ok(Some(vec![0.5; 100]))
            } else {
                Err(AppError::audio("injected microphone failure"))
            }
        },
        clock_at(
            started_at,
            &[Duration::ZERO, Duration::ZERO, Duration::from_millis(100)],
        ),
    )
    .err();

    assert_eq!(
        error.map(|error| error.to_string()),
        Some("injected microphone failure".to_string())
    );
}

#[test]
fn result_serializes_as_six_scalars_without_audio() -> AppResult<()> {
    let value = serde_json::to_value(AudioProbeResult {
        sample_rate: 48_000,
        duration_ms: 2_000,
        rms_dbfs: -12.0,
        peak_dbfs: -3.0,
        clipping: false,
        gate_open: true,
    })
    .map_err(|error| AppError::audio(format!("Failed to serialize probe result: {error}")))?;

    assert_eq!(
        value,
        serde_json::json!({
            "sampleRate": 48_000,
            "durationMs": 2_000,
            "rmsDbfs": -12.0,
            "peakDbfs": -3.0,
            "clipping": false,
            "gateOpen": true,
        })
    );
    Ok(())
}
