use super::super::layout::predict_layout;
use super::support::{
    CHATBOX_REGRESSION_CORPUS_JSON, PREPARATION_POLICY_EXPECTATIONS,
    VRCHAT_CLIENT_OBSERVATIONS_JSON, require_object_fields, required_string, required_usize,
    required_usize_array,
};
use serde_json::Value;
use std::collections::HashSet;

const EXPECTED_VRCHAT_CLIENT_OBSERVATION_COUNT: usize = 52;

#[test]
fn build_scoped_vrchat_client_observations_match_layout_predictions() -> Result<(), String> {
    let corpus = serde_json::from_str::<Value>(CHATBOX_REGRESSION_CORPUS_JSON)
        .map_err(|error| error.to_string())?;
    let observations = serde_json::from_str::<Value>(VRCHAT_CLIENT_OBSERVATIONS_JSON)
        .map_err(|error| error.to_string())?;
    require_object_fields(
        &observations,
        "VRChat client-observation fixture",
        &[
            "description",
            "field_semantics",
            "fixture_id",
            "observation_schema_version",
            "observations",
            "schema_version_scope",
            "vrchat_profile",
        ],
        &[],
    )?;
    require_object_fields(
        &observations["vrchat_profile"],
        "VRChat observation profile",
        &[
            "chat_bubble_opacity",
            "chat_bubble_scale",
            "distribution",
            "platform",
            "unity_version",
            "vrchat_build",
            "xr_device",
        ],
        &[],
    )?;
    require_object_fields(
        &observations["field_semantics"],
        "VRChat observation field semantics",
        &[
            "soft_wrap_utf16_offsets",
            "visible_line_clipping",
            "visual_line_count",
        ],
        &[],
    )?;
    require_object_fields(
        &observations["observations"],
        "VRChat observation groups",
        &[
            "cjk_kinsoku",
            "control_characters",
            "visible_line_limit",
            "width_calibration",
        ],
        &[],
    )?;
    let cases = corpus["cases"]
        .as_array()
        .ok_or("Chatbox corpus cases must be an array.")?;
    let cases_by_id = cases
        .iter()
        .map(|case| Ok((required_string(case, "case_id")?, case)))
        .collect::<Result<std::collections::HashMap<_, _>, String>>()?;

    assert_eq!(observations["observation_schema_version"], 1);
    assert_eq!(observations["schema_version_scope"], "test_fixture_only");
    assert_eq!(
        observations["fixture_id"],
        "vrchat-chatbox-client-observations-2026.3.1-1885-81193b80fa"
    );
    assert_eq!(
        observations["vrchat_profile"]["vrchat_build"],
        "2026.3.1-1885-81193b80fa-Release"
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
            require_object_fields(
                observation,
                &format!("VRChat client observation {case_id}"),
                &["case_id", "payload_sha256", "visual_line_count"],
                &["soft_wrap_utf16_offsets", "visible_line_clipping"],
            )?;
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
            let observed_line_count = required_usize(observation, "visual_line_count")?;
            let observed_soft_wrap_offsets = observation
                .get("soft_wrap_utf16_offsets")
                .map(|_| required_usize_array(observation, "soft_wrap_utf16_offsets"))
                .transpose()?;
            let observed_clipping = observation
                .get("visible_line_clipping")
                .map(|value| {
                    value.as_bool().ok_or_else(|| {
                        format!("visible_line_clipping must be a Boolean: {case_id}")
                    })
                })
                .transpose()?;

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
                observed_line_count,
                "layout prediction line count differs from VRChat client observation: {case_id}"
            );
            if let Some(observed_soft_wrap_offsets) = observed_soft_wrap_offsets {
                assert_eq!(
                    prediction.soft_break_utf16_offsets(),
                    observed_soft_wrap_offsets,
                    "layout prediction soft breaks differ from VRChat client observation: {case_id}"
                );
            }
            if let Some(observed_clipping) = observed_clipping {
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
    Ok(())
}
