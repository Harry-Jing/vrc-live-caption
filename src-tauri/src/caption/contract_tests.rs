use super::*;

#[test]
fn shared_v1_fixture_round_trips_without_a_stable_state() -> Result<(), serde_json::Error> {
    let fixture = include_str!("../../../contracts/caption-aggregate-snapshot-v1.json");
    let expected = serde_json::from_str::<serde_json::Value>(fixture)?;
    let snapshot = serde_json::from_str::<CaptionAggregateSnapshot>(fixture)?;
    let actual = serde_json::to_value(snapshot)?;

    assert_eq!(actual, expected);
    assert_eq!(actual["contractVersion"], 1);
    assert!(!fixture.contains("stable"));

    Ok(())
}
