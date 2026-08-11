use super::*;

#[test]
fn shared_v2_fixture_round_trips_without_a_stable_state() -> Result<(), serde_json::Error> {
    let fixture = include_str!("../../../contracts/caption-aggregate-snapshot-v2.json");
    let expected = serde_json::from_str::<serde_json::Value>(fixture)?;
    let snapshot = serde_json::from_str::<CaptionAggregateSnapshotV2>(fixture)?;
    let actual = serde_json::to_value(snapshot)?;

    assert_eq!(actual, expected);
    assert!(!fixture.contains("stable"));

    Ok(())
}
