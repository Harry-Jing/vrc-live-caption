use super::*;

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 0.001,
        "expected {actual} to be close to {expected}"
    );
}

fn readings_for_partitions(
    samples: &[f32],
    partitions: &[usize],
) -> Result<Vec<AudioLevelReading>, AudioLevelConfigError> {
    let mut meter = AudioLevelMeter::new(1_000, 0.25)?;
    let mut readings = Vec::new();
    let mut offset = 0;

    for partition in partitions.iter().copied().cycle() {
        if offset >= samples.len() {
            break;
        }
        let end = offset.saturating_add(partition).min(samples.len());
        readings.extend(meter.push_samples(&samples[offset..end]));
        offset = end;
    }

    Ok(readings)
}

#[test]
fn callback_partitioning_does_not_change_fixed_window_readings() -> Result<(), AudioLevelConfigError>
{
    let mut samples = vec![0.5; 100];
    samples.extend(vec![0.1; 100]);
    samples.extend(vec![0.8; 50]);

    let grouped = readings_for_partitions(&samples, &[samples.len()])?;
    let per_sample = readings_for_partitions(&samples, &[1])?;
    let irregular = readings_for_partitions(&samples, &[7, 31, 2, 43])?;

    assert_eq!(grouped, per_sample);
    assert_eq!(grouped, irregular);
    assert_eq!(grouped.len(), 2);
    assert!(grouped[0].vad_gate_open);
    assert!(!grouped[1].vad_gate_open);
    Ok(())
}

#[test]
fn silence_uses_a_finite_serializable_dbfs_floor() -> Result<(), AudioLevelConfigError> {
    let mut meter = AudioLevelMeter::new(48_000, 0.012)?;

    let readings = meter.push_samples(&vec![0.0; 4_800]);

    assert_eq!(readings.len(), 1);
    assert_eq!(readings[0].rms_dbfs, DBFS_FLOOR);
    assert_eq!(readings[0].peak_dbfs, DBFS_FLOOR);
    assert!(readings[0].rms_dbfs.is_finite());
    assert!(readings[0].peak_dbfs.is_finite());
    assert!(!readings[0].clipping);
    assert!(!readings[0].vad_gate_open);
    Ok(())
}

#[test]
fn full_scale_sample_marks_the_window_as_clipping() -> Result<(), AudioLevelConfigError> {
    let mut meter = AudioLevelMeter::new(1_000, 0.012)?;
    let mut samples = vec![0.5; 100];
    samples[37] = -1.0;

    let readings = meter.push_samples(&samples);

    assert_eq!(readings.len(), 1);
    assert_eq!(readings[0].peak_dbfs, 0.0);
    assert!(readings[0].clipping);
    assert!(readings[0].vad_gate_open);
    Ok(())
}

#[test]
fn complete_windows_emit_and_remainders_carry_into_the_next_callback()
-> Result<(), AudioLevelConfigError> {
    let mut meter = AudioLevelMeter::new(1_000, 0.25)?;

    assert!(meter.push_samples(&vec![0.5; 99]).is_empty());
    let boundary = meter.push_samples(&[0.5]);
    assert_eq!(boundary.len(), 1);
    assert!(boundary[0].vad_gate_open);

    let two_windows_and_half = meter.push_samples(&vec![0.1; 250]);
    assert_eq!(two_windows_and_half.len(), 2);
    assert!(
        two_windows_and_half
            .iter()
            .all(|reading| !reading.vad_gate_open)
    );
    assert!(meter.push_samples(&[0.1; 49]).is_empty());

    let completed_remainder = meter.push_samples(&[0.1]);
    assert_eq!(completed_remainder.len(), 1);
    assert!(!completed_remainder[0].vad_gate_open);
    Ok(())
}

#[test]
fn constant_signal_reports_rms_peak_and_the_configured_gate() -> Result<(), AudioLevelConfigError> {
    let mut meter = AudioLevelMeter::new(1_000, 0.012)?;

    let loud = meter.push_samples(&vec![0.5; 100]);
    let at_gate = meter.push_samples(&vec![0.012; 100]);
    let below_gate = meter.push_samples(&vec![0.011; 100]);

    assert_close(loud[0].rms_dbfs, -6.0206);
    assert_close(loud[0].peak_dbfs, -6.0206);
    assert!(at_gate[0].vad_gate_open);
    assert!(!below_gate[0].vad_gate_open);
    Ok(())
}

#[test]
fn gate_reports_any_ten_millisecond_frame_that_crosses_the_vad_threshold()
-> Result<(), AudioLevelConfigError> {
    let mut meter = AudioLevelMeter::new(1_000, 0.012)?;
    let mut samples = vec![0.02; 10];
    samples.extend(vec![0.0; 90]);

    let readings = meter.push_samples(&samples);

    assert_eq!(readings.len(), 1);
    assert!(readings[0].rms_dbfs < amplitude_to_dbfs(0.012));
    assert!(readings[0].vad_gate_open);
    Ok(())
}

#[test]
fn non_finite_pcm_cannot_create_non_finite_statistics() -> Result<(), AudioLevelConfigError> {
    let mut meter = AudioLevelMeter::new(1_000, 0.012)?;
    let mut samples = vec![0.0; 100];
    samples[10] = f32::NAN;
    samples[20] = f32::INFINITY;
    samples[30] = f32::NEG_INFINITY;

    let readings = meter.push_samples(&samples);

    assert_eq!(readings.len(), 1);
    assert!(readings[0].rms_dbfs.is_finite());
    assert!(readings[0].peak_dbfs.is_finite());
    assert!(readings[0].clipping);
    Ok(())
}

#[test]
fn invalid_sample_rate_and_gate_are_rejected() {
    assert_eq!(
        AudioLevelMeter::new(0, 0.012).err(),
        Some(AudioLevelConfigError::ZeroSampleRate)
    );
    for threshold in [0.0, -0.1, 1.1, f32::NAN, f32::INFINITY] {
        assert_eq!(
            AudioLevelMeter::new(48_000, threshold).err(),
            Some(AudioLevelConfigError::InvalidGateThreshold)
        );
    }
}

#[test]
fn window_sample_count_is_derived_from_the_actual_sample_rate() -> Result<(), AudioLevelConfigError>
{
    let mut meter = AudioLevelMeter::new(44_101, 0.012)?;

    assert!(meter.push_samples(&vec![0.1; 4_410]).is_empty());
    assert_eq!(meter.push_samples(&[0.1]).len(), 1);
    Ok(())
}
