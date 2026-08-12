use super::*;

#[test]
fn shared_v2_fixture_round_trips_with_translation_outcomes() -> Result<(), serde_json::Error> {
    let fixture = include_str!("../../../contracts/caption-aggregate-snapshot-v2.json");
    let expected = serde_json::from_str::<serde_json::Value>(fixture)?;
    let snapshot = serde_json::from_str::<CaptionAggregateSnapshot>(fixture)?;
    let actual = serde_json::to_value(snapshot)?;

    assert_eq!(actual, expected);
    assert_eq!(actual["contractVersion"], 2);
    assert_eq!(actual["translationUnits"].as_array().map(Vec::len), Some(3));
    assert!(!fixture.contains("stable"));

    Ok(())
}

#[test]
fn translation_failure_reasons_match_the_shared_closed_vocabulary() -> Result<(), serde_json::Error>
{
    let vocabulary = serde_json::from_str::<serde_json::Value>(include_str!(
        "../../../contracts/wire-vocabulary.json"
    ))?;
    let reasons = serde_json::to_value([
        TranslationFailureReason::ProviderAuthenticationFailed,
        TranslationFailureReason::ProviderPermissionDenied,
        TranslationFailureReason::ProviderInvalidRequest,
        TranslationFailureReason::ProviderRateLimited,
        TranslationFailureReason::ProviderUsageLimit,
        TranslationFailureReason::ProviderUnavailable,
        TranslationFailureReason::InvalidOutput,
        TranslationFailureReason::DeadlineExceeded,
        TranslationFailureReason::Backpressure,
        TranslationFailureReason::SourceTooLarge,
        TranslationFailureReason::Stopped,
        TranslationFailureReason::Failed,
    ])?;

    assert_eq!(reasons, vocabulary["translationFailureReasons"]);

    Ok(())
}
