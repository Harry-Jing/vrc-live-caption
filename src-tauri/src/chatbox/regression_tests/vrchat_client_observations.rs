use super::super::layout::predict_layout;
use super::support::{
    EXPECTED_MANIFEST_SHA256, EXPECTED_SOURCE_SHA256, PORTABLE_CORPUS_JSON,
    PREPARATION_POLICY_EXPECTATIONS, VRCHAT_CLIENT_OBSERVATIONS_JSON,
    assert_no_forbidden_expectation_fields, required_string, required_usize, required_usize_array,
    sha256_hex,
};
use serde_json::Value;
use std::collections::HashSet;

const EXPECTED_VRCHAT_CLIENT_OBSERVATION_COUNT: usize = 52;

#[test]
fn build_scoped_vrchat_client_observations_match_layout_predictions() -> Result<(), String> {
    let corpus =
        serde_json::from_str::<Value>(PORTABLE_CORPUS_JSON).map_err(|error| error.to_string())?;
    let observations = serde_json::from_str::<Value>(VRCHAT_CLIENT_OBSERVATIONS_JSON)
        .map_err(|error| error.to_string())?;
    let cases = corpus["cases"]
        .as_array()
        .ok_or("Chatbox corpus cases must be an array.")?;
    let cases_by_id = cases
        .iter()
        .map(|case| Ok((required_string(case, "case_id")?, case)))
        .collect::<Result<std::collections::HashMap<_, _>, String>>()?;

    assert_eq!(observations["observation_schema_version"], 1);
    assert_eq!(
        observations["evidence_class"],
        "runtime_screenshot_observation"
    );
    assert_eq!(
        observations["runtime_profile"]["vrchat_build"],
        "2026.3.1-1885-81193b80fa-Release"
    );
    assert_eq!(
        observations["provenance"]["portable_corpus"]["source_sha256"],
        EXPECTED_SOURCE_SHA256
    );
    assert_eq!(
        observations["provenance"]["portable_corpus"]["manifest_sha256"],
        EXPECTED_MANIFEST_SHA256
    );
    assert_eq!(
        observations["provenance"]["portable_corpus"]["portable_fixture_sha256"],
        sha256_hex(PORTABLE_CORPUS_JSON.as_bytes())
    );
    assert_eq!(
        observations["provenance"]["evidence_artifacts"]["run_sha256"],
        "cba9aa7f762678a489211f7a2801a2653b5f071035525e28f566a3ffe96b4055"
    );
    assert_eq!(
        observations["provenance"]["evidence_artifacts"]["analysis_sha256"],
        "af1fcf39d4e26cc548ac0d67ae6fb94fd7ba7ed58364873d6f079de7617bef02"
    );
    assert_eq!(
        observations["selection"]["case_count"],
        EXPECTED_VRCHAT_CLIENT_OBSERVATION_COUNT
    );
    assert_eq!(observations["selection"]["layout_trace_oracle_count"], 49);
    assert_eq!(
        observations["selection"]["preparation_policy_evidence_count"],
        PREPARATION_POLICY_EXPECTATIONS.len()
    );

    let groups = observations["observations"]
        .as_object()
        .ok_or("VRChat client observations must be grouped by purpose")?;
    let mut observed_count = 0;
    let mut compared_count = 0;
    let mut observed_ids = HashSet::new();
    for group in groups.values() {
        let group = group
            .as_array()
            .ok_or("each VRChat client observation group must be an array")?;
        for observation in group {
            observed_count += 1;
            let case_id = required_string(observation, "case_id")?;
            assert!(
                observed_ids.insert(case_id),
                "duplicate VRChat client observation: {case_id}"
            );
            let case = cases_by_id.get(case_id).ok_or_else(|| {
                format!("VRChat client observation has no corpus case: {case_id}")
            })?;
            assert_eq!(
                required_string(observation, "payload_sha256")?,
                required_string(case, "payload_sha256")?,
                "VRChat client observation payload identity drifted: {case_id}"
            );

            // The raw CR/NEL observations explain why product preparation is
            // necessary. Their observed geometry is not an expectation for the
            // deliberately transformed outgoing string.
            if PREPARATION_POLICY_EXPECTATIONS
                .iter()
                .any(|(override_case_id, _, _)| *override_case_id == case_id)
            {
                continue;
            }
            compared_count += 1;

            let prediction = predict_layout(required_string(case, "payload")?)
                .map_err(|error| format!("layout prediction rejected {case_id}: {error:?}"))?;
            assert_eq!(
                prediction.visible_line_count(),
                required_usize(observation, "visual_line_count")?,
                "layout prediction line count differs from VRChat client observation: {case_id}"
            );
            if observation.get("soft_wrap_utf16_offsets").is_some() {
                assert_eq!(
                    prediction.soft_break_utf16_offsets(),
                    required_usize_array(observation, "soft_wrap_utf16_offsets")?,
                    "layout prediction soft breaks differ from VRChat client observation: {case_id}"
                );
            }
            if let Some(observed_clipping) = observation
                .get("visible_line_clipping")
                .and_then(Value::as_bool)
            {
                assert_eq!(
                    prediction.is_clipped(),
                    observed_clipping,
                    "layout prediction clipping differs from VRChat client observation: {case_id}"
                );
            }
        }
    }

    assert_eq!(observed_count, EXPECTED_VRCHAT_CLIENT_OBSERVATION_COUNT);
    assert_eq!(compared_count, EXPECTED_VRCHAT_CLIENT_OBSERVATION_COUNT - 3);
    assert_no_forbidden_expectation_fields(&observations);
    assert!(!VRCHAT_CLIENT_OBSERVATIONS_JSON.contains("\"payload\""));
    assert!(!VRCHAT_CLIENT_OBSERVATIONS_JSON.contains("C:\\\\Users\\\\"));
    Ok(())
}
