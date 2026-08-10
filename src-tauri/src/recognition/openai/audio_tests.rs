use super::*;
use crate::error::AppResult;

fn decode_pcm16(bytes: &[u8]) -> AppResult<Vec<i16>> {
    bytes
        .chunks_exact(2)
        .map(|chunk| {
            let array = <[u8; 2]>::try_from(chunk)
                .map_err(|_| AppError::state("PCM16 test received a partial sample."))?;
            Ok(i16::from_le_bytes(array))
        })
        .collect()
}

#[test]
fn target_rate_is_encoded_as_signed_little_endian_pcm16() -> AppResult<()> {
    let mut encoder = RealtimePcm16Encoder::new();
    let bytes = encoder.append(24_000, &[0.0, 1.0, -1.0, 2.0, -2.0])?;
    assert_eq!(
        decode_pcm16(&bytes)?,
        vec![0, i16::MAX, -i16::MAX, i16::MAX, -i16::MAX]
    );
    assert!(encoder.finish_unit().is_empty());
    Ok(())
}

#[test]
fn resampling_is_stable_across_capture_chunk_boundaries() -> AppResult<()> {
    let samples = [0.0, 0.25, 0.5, 0.75, 1.0, 0.5, 0.0, -0.5];

    let mut whole = RealtimePcm16Encoder::new();
    let mut expected = whole.append(48_000, &samples)?;
    expected.extend(whole.finish_unit());

    let mut chunked = RealtimePcm16Encoder::new();
    let mut actual = chunked.append(48_000, &samples[..3])?;
    actual.extend(chunked.append(48_000, &samples[3..6])?);
    actual.extend(chunked.append(48_000, &samples[6..])?);
    actual.extend(chunked.finish_unit());

    assert_eq!(actual, expected);
    assert_eq!(decode_pcm16(&actual)?.len(), 4);
    Ok(())
}

#[test]
fn unsupported_rate_changes_and_non_finite_samples_are_rejected() -> AppResult<()> {
    let mut encoder = RealtimePcm16Encoder::new();
    let _ = encoder.append(48_000, &[0.0, 0.1])?;
    assert!(encoder.append(44_100, &[0.2]).is_err());

    let mut non_finite = RealtimePcm16Encoder::new();
    assert!(non_finite.append(24_000, &[f32::NAN]).is_err());
    Ok(())
}
