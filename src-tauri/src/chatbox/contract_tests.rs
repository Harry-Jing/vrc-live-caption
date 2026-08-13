use super::layout::{
    CHATBOX_MAX_UTF16_UNITS, ChatboxLayoutError, paginate_completed, prepare_single_message,
    render_live_viewport, trace_layout,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::collections::HashSet;
use unicode_segmentation::UnicodeSegmentation;

const FIXTURE: &str = include_str!("../../../contracts/chatbox-layout-cases-v1.json");
const RUNTIME_OBSERVATIONS: &str = include_str!(
    "../../../contracts/chatbox-layout-runtime-observations-2026.3.1-1885-81193b80fa-v1.json"
);
const EXPECTED_CASE_COUNT: usize = 178;
const EXPECTED_RUNTIME_OBSERVATION_COUNT: usize = 52;
const EXPECTED_COMPLETED_TARGET_COUNT: usize = 98;
const EXPECTED_LIVE_TARGET_COUNT: usize = 96;
const EXPECTED_SOURCE_SHA256: &str =
    "f4899d95d0a2fac74a96423608cd4d9b88fa3afe28737c747356fcd3d4190731";
const EXPECTED_MANIFEST_SHA256: &str =
    "fcbad159f2fb9ea6f0b9bd9fab33373c913d48e7b3213913bb73c8a3ac68e7d8";
const ALLOWED_TEST_TARGETS: [&str; 4] = [
    "completed-pagination",
    "layout",
    "live-window",
    "sender-policy",
];
const FORBIDDEN_GENERATED_EXPECTATION_FIELDS: [&str; 8] = [
    "expected_pages",
    "model_prediction",
    "observed_break_utf16_offsets",
    "observed_line_count",
    "runtime_observation",
    "screenshot",
    "screenshot_sha256",
    "vrchat_version",
];

// These are product-policy expectations, not observations copied from a
// VRChat build. Keeping the small set explicit makes a newly targeted control
// character fail closed until its intended preparation is reviewed.
const PREPARED_PAYLOAD_OVERRIDES: [(&str, &str, &str); 3] = [
    ("LINES-CR-BASIC", "alpha\rbeta", "alpha beta"),
    ("LINES-NEL-BASIC", "alpha\u{0085}beta", "alpha beta"),
    (
        "LINES-NINE-CR",
        "one\rtwo\rthree\rfour\rfive\rsix\rseven\reight\rnine",
        "one two three four five six seven eight nine",
    ),
];

#[test]
fn portable_chatbox_corpus_has_stable_identity_and_unicode_facts()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = serde_json::from_str::<Value>(FIXTURE)?;
    let cases = fixture["cases"]
        .as_array()
        .ok_or("Chatbox fixture cases must be an array.")?;

    assert_eq!(fixture["fixture_schema_version"], 1);
    assert_eq!(fixture["case_count"], EXPECTED_CASE_COUNT);
    assert_eq!(cases.len(), EXPECTED_CASE_COUNT);
    assert_eq!(fixture["source"]["corpus_id"], "vrchat-chatbox-canonical");
    assert_eq!(fixture["source"]["source_sha256"], EXPECTED_SOURCE_SHA256);
    assert_eq!(
        fixture["source"]["manifest_sha256"],
        EXPECTED_MANIFEST_SHA256
    );
    assert_eq!(fixture["source"]["unicode_profile"]["version"], "17.0.0");
    assert_no_forbidden_expectation_fields(&fixture);

    let mut case_ids = HashSet::new();
    let mut payloads = HashSet::new();
    for case in cases {
        let case_id = required_string(case, "case_id")?;
        let payload = required_string(case, "payload")?;
        let facts = &case["portable_unicode_facts"];

        assert!(case_ids.insert(case_id), "duplicate case ID: {case_id}");
        assert!(
            payloads.insert(payload),
            "duplicate payload for case: {case_id}"
        );
        assert!(
            !payload.contains(case_id),
            "case ID entered transmitted payload: {case_id}"
        );
        assert!(
            case_id
                .chars()
                .all(|character| character.is_ascii_uppercase()
                    || character.is_ascii_digit()
                    || character == '-'),
            "case ID is not path-safe: {case_id}"
        );
        assert!(!payload.contains('\0'), "payload contains NUL: {case_id}");
        let targets = case["test_targets"]
            .as_array()
            .ok_or_else(|| format!("test_targets must be an array: {case_id}"))?;
        assert!(
            !targets.is_empty(),
            "case has no production target: {case_id}"
        );
        assert!(targets.iter().all(|target| {
            target
                .as_str()
                .is_some_and(|target| ALLOWED_TEST_TARGETS.contains(&target))
        }));
        assert_eq!(
            required_string(case, "payload_sha256")?,
            sha256_hex(payload.as_bytes()),
            "payload SHA-256 drifted: {case_id}"
        );
        assert_eq!(facts["evidence_class"], "deterministic_unicode_computation");
        assert_eq!(required_usize(facts, "utf8_bytes")?, payload.len());
        assert_eq!(
            required_usize(facts, "utf16_units")?,
            payload.encode_utf16().count()
        );
        assert_eq!(
            required_usize(facts, "code_points")?,
            payload.chars().count()
        );

        let graphemes = payload.graphemes(true).collect::<Vec<_>>();
        assert_eq!(required_usize(facts, "graphemes")?, graphemes.len());
        assert_eq!(
            required_usize_array(case, "egc_end_utf16_offsets")?,
            egc_end_utf16_offsets(&graphemes)
        );

        let safe_prefix = required_string(case, "egc_safe_prefix_144")?;
        assert!(payload.starts_with(safe_prefix));
        assert!(safe_prefix.encode_utf16().count() <= 144);
        assert_eq!(
            required_usize(&facts["egc_safe_prefix_144"], "utf16_units")?,
            safe_prefix.encode_utf16().count()
        );
        assert_eq!(
            required_usize(&facts["egc_safe_prefix_144"], "graphemes")?,
            safe_prefix.graphemes(true).count()
        );
        assert_eq!(
            facts["egc_safe_prefix_144"]["equals_payload"],
            safe_prefix == payload
        );
        let next_grapheme = payload
            .get(safe_prefix.len()..)
            .and_then(|suffix| suffix.graphemes(true).next());
        assert!(next_grapheme.is_none_or(|grapheme| {
            safe_prefix.encode_utf16().count() + grapheme.encode_utf16().count() > 144
        }));
    }

    assert!(!FIXTURE.contains("C:\\\\Users\\\\Harry\\\\"));
    assert!(!FIXTURE.contains("vrc-chatbox-layout-lab"));
    assert!(!FIXTURE.contains("captures/"));
    assert!(!FIXTURE.contains("captures\\\\"));
    Ok(())
}

#[test]
fn completed_targets_form_lossless_standalone_prepared_pages() -> Result<(), String> {
    let fixture = serde_json::from_str::<Value>(FIXTURE).map_err(|error| error.to_string())?;
    let cases = fixture["cases"]
        .as_array()
        .ok_or("Chatbox fixture cases must be an array.")?;
    let mut target_count = 0;
    let mut oversized_count = 0;

    for case in cases {
        if !has_test_target(case, "completed-pagination")? {
            continue;
        }
        target_count += 1;

        let case_id = required_string(case, "case_id")?;
        let payload = required_string(case, "payload")?;
        let prepared_source = expected_prepared_payload(case_id, payload)?;
        let prepared_source = prepared_source.as_ref();
        let fixture_egc_ends = required_usize_array(case, "egc_end_utf16_offsets")?;
        assert_preparation_preserves_fixture_boundaries(case_id, case, prepared_source)?;

        let result = paginate_completed(payload);
        if let Some(utf16_units) = first_oversized_grapheme_utf16_units(prepared_source) {
            oversized_count += 1;
            assert_eq!(
                result,
                Err(ChatboxLayoutError::GraphemeExceedsInputBudget { utf16_units }),
                "Completed returned the wrong oversized-EGC error for {case_id}"
            );
            continue;
        }

        let pages = result.map_err(|error| {
            format!("Completed rejected representable fixture case {case_id}: {error:?}")
        })?;
        if prepared_source.is_empty() {
            assert!(pages.is_empty(), "empty case emitted a page: {case_id}");
            continue;
        }

        assert!(
            !pages.is_empty(),
            "nonempty case emitted no pages: {case_id}"
        );
        assert_eq!(
            pages.iter().map(|page| page.as_str()).collect::<String>(),
            prepared_source,
            "Completed pages did not losslessly partition prepared text: {case_id}"
        );

        let mut consumed_utf16_units = 0;
        for page in &pages {
            let page = page.as_str();
            assert!(
                !page.is_empty(),
                "Completed emitted an empty page: {case_id}"
            );
            let page_utf16_units = page.encode_utf16().count();
            assert!(
                page_utf16_units <= CHATBOX_MAX_UTF16_UNITS,
                "Completed page exceeded the input budget for {case_id}: {page_utf16_units}"
            );
            consumed_utf16_units += page_utf16_units;
            assert!(
                fixture_egc_ends
                    .binary_search(&consumed_utf16_units)
                    .is_ok(),
                "Completed page ended inside an EGC for {case_id}: {consumed_utf16_units}"
            );

            let standalone = prepare_single_message(page).map_err(|error| {
                format!("Completed page was not standalone-safe for {case_id}: {error:?}")
            })?;
            let standalone = standalone
                .ok_or_else(|| format!("Completed page disappeared when re-prepared: {case_id}"))?;
            assert_eq!(
                standalone.as_str(),
                page,
                "Completed page changed when prepared independently: {case_id}"
            );
        }
    }

    assert_eq!(target_count, EXPECTED_COMPLETED_TARGET_COUNT);
    assert_eq!(
        oversized_count, 2,
        "portable corpus lost oversized EGC coverage"
    );
    Ok(())
}

#[test]
fn live_targets_form_bounded_newest_standalone_suffixes() -> Result<(), String> {
    let fixture = serde_json::from_str::<Value>(FIXTURE).map_err(|error| error.to_string())?;
    let cases = fixture["cases"]
        .as_array()
        .ok_or("Chatbox fixture cases must be an array.")?;
    let mut target_count = 0;
    let mut oversized_count = 0;
    let mut oversized_error_count = 0;

    for case in cases {
        if !has_test_target(case, "live-window")? {
            continue;
        }
        target_count += 1;

        let case_id = required_string(case, "case_id")?;
        let payload = required_string(case, "payload")?;
        let prepared_source = expected_prepared_payload(case_id, payload)?;
        let prepared_source = prepared_source.as_ref();
        let fixture_egc_ends = required_usize_array(case, "egc_end_utf16_offsets")?;
        assert_preparation_preserves_fixture_boundaries(case_id, case, prepared_source)?;

        let newest_oversized = newest_oversized_grapheme(prepared_source);
        oversized_count += usize::from(newest_oversized.is_some());
        let result = render_live_viewport(payload);
        if let Some((end, utf16_units)) = newest_oversized
            && end == prepared_source.len()
        {
            oversized_error_count += 1;
            assert_eq!(
                result,
                Err(ChatboxLayoutError::GraphemeExceedsInputBudget { utf16_units }),
                "Live returned the wrong newest oversized-EGC error for {case_id}"
            );
            continue;
        }

        let viewport = result.map_err(|error| {
            format!("Live rejected a case with a representable newest suffix {case_id}: {error:?}")
        })?;
        if prepared_source.is_empty() {
            assert!(
                viewport.is_none(),
                "empty case emitted a Live viewport: {case_id}"
            );
            continue;
        }

        let viewport =
            viewport.ok_or_else(|| format!("nonempty case emitted no Live viewport: {case_id}"))?;
        let viewport = viewport.as_str();
        assert!(
            !viewport.is_empty(),
            "Live emitted an empty viewport: {case_id}"
        );
        let viewport_utf16_units = viewport.encode_utf16().count();
        assert!(
            viewport_utf16_units <= CHATBOX_MAX_UTF16_UNITS,
            "Live viewport exceeded the input budget for {case_id}: {viewport_utf16_units}"
        );
        assert!(
            prepared_source.ends_with(viewport),
            "Live viewport was not a suffix of prepared text: {case_id}"
        );
        assert_eq!(
            viewport.graphemes(true).next_back(),
            prepared_source.graphemes(true).next_back(),
            "Live viewport did not retain the newest EGC: {case_id}"
        );
        if let Ok(Some(single_view)) = prepare_single_message(prepared_source) {
            assert_eq!(
                viewport,
                single_view.as_str(),
                "Live discarded text even though the full prepared source was safe: {case_id}"
            );
        }

        let viewport_start_utf16_units =
            prepared_source.encode_utf16().count() - viewport_utf16_units;
        assert!(
            viewport_start_utf16_units == 0
                || fixture_egc_ends
                    .binary_search(&viewport_start_utf16_units)
                    .is_ok(),
            "Live viewport started inside an EGC for {case_id}: {viewport_start_utf16_units}"
        );

        let standalone = prepare_single_message(viewport).map_err(|error| {
            format!("Live viewport was not standalone-safe for {case_id}: {error:?}")
        })?;
        let standalone = standalone
            .ok_or_else(|| format!("Live viewport disappeared when re-prepared: {case_id}"))?;
        assert_eq!(
            standalone.as_str(),
            viewport,
            "Live viewport changed when prepared independently: {case_id}"
        );
    }

    assert_eq!(target_count, EXPECTED_LIVE_TARGET_COUNT);
    assert_eq!(
        oversized_count, 2,
        "portable corpus lost oversized EGC coverage"
    );
    assert_eq!(
        oversized_error_count, 1,
        "portable corpus must distinguish old from newest oversized EGCs"
    );
    Ok(())
}

#[test]
fn build_scoped_runtime_observations_match_the_layout_trace() -> Result<(), String> {
    let portable = serde_json::from_str::<Value>(FIXTURE).map_err(|error| error.to_string())?;
    let observations =
        serde_json::from_str::<Value>(RUNTIME_OBSERVATIONS).map_err(|error| error.to_string())?;
    let cases = portable["cases"]
        .as_array()
        .ok_or("Chatbox fixture cases must be an array.")?;
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
        sha256_hex(FIXTURE.as_bytes())
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
        EXPECTED_RUNTIME_OBSERVATION_COUNT
    );
    assert_eq!(observations["selection"]["layout_trace_oracle_count"], 49);
    assert_eq!(
        observations["selection"]["preparation_policy_evidence_count"],
        PREPARED_PAYLOAD_OVERRIDES.len()
    );

    let groups = observations["observations"]
        .as_object()
        .ok_or("runtime observations must be grouped by purpose")?;
    let mut observed_count = 0;
    let mut compared_count = 0;
    let mut observed_ids = HashSet::new();
    for group in groups.values() {
        let group = group
            .as_array()
            .ok_or("each runtime observation group must be an array")?;
        for observation in group {
            observed_count += 1;
            let case_id = required_string(observation, "case_id")?;
            assert!(
                observed_ids.insert(case_id),
                "duplicate runtime observation: {case_id}"
            );
            let case = cases_by_id
                .get(case_id)
                .ok_or_else(|| format!("runtime observation has no portable case: {case_id}"))?;
            assert_eq!(
                required_string(observation, "payload_sha256")?,
                required_string(case, "payload_sha256")?,
                "runtime observation payload identity drifted: {case_id}"
            );

            // The raw CR/NEL observations explain why product preparation is
            // necessary. Their observed geometry is not an expectation for the
            // deliberately transformed outgoing string.
            if PREPARED_PAYLOAD_OVERRIDES
                .iter()
                .any(|(override_case_id, _, _)| *override_case_id == case_id)
            {
                continue;
            }
            compared_count += 1;

            let trace = trace_layout(required_string(case, "payload")?)
                .map_err(|error| format!("layout trace rejected {case_id}: {error:?}"))?;
            assert_eq!(
                trace.visible_line_count(),
                required_usize(observation, "visual_line_count")?,
                "layout trace line count differs from runtime observation: {case_id}"
            );
            if observation.get("soft_wrap_utf16_offsets").is_some() {
                assert_eq!(
                    trace.soft_break_utf16_offsets(),
                    required_usize_array(observation, "soft_wrap_utf16_offsets")?,
                    "layout trace soft breaks differ from runtime observation: {case_id}"
                );
            }
            if let Some(observed_clipping) = observation
                .get("visible_line_clipping")
                .and_then(Value::as_bool)
            {
                assert_eq!(
                    trace.clipped(),
                    observed_clipping,
                    "layout trace clipping differs from runtime observation: {case_id}"
                );
            }
        }
    }

    assert_eq!(observed_count, EXPECTED_RUNTIME_OBSERVATION_COUNT);
    assert_eq!(compared_count, EXPECTED_RUNTIME_OBSERVATION_COUNT - 3);
    assert_no_forbidden_expectation_fields(&observations);
    assert!(!RUNTIME_OBSERVATIONS.contains("\"payload\""));
    assert!(!RUNTIME_OBSERVATIONS.contains("C:\\\\Users\\\\"));
    Ok(())
}

fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value[field]
        .as_str()
        .ok_or_else(|| format!("{field} must be a string."))
}

fn required_usize(value: &Value, field: &str) -> Result<usize, String> {
    value[field]
        .as_u64()
        .and_then(|number| usize::try_from(number).ok())
        .ok_or_else(|| format!("{field} must be a non-negative integer."))
}

fn required_usize_array(value: &Value, field: &str) -> Result<Vec<usize>, String> {
    value[field]
        .as_array()
        .ok_or_else(|| format!("{field} must be an array."))?
        .iter()
        .map(|entry| {
            entry
                .as_u64()
                .and_then(|number| usize::try_from(number).ok())
                .ok_or_else(|| format!("{field} entries must be non-negative integers."))
        })
        .collect()
}

fn has_test_target(case: &Value, expected: &str) -> Result<bool, String> {
    let case_id = required_string(case, "case_id")?;
    let targets = case["test_targets"]
        .as_array()
        .ok_or_else(|| format!("test_targets must be an array: {case_id}"))?;

    Ok(targets
        .iter()
        .any(|target| target.as_str() == Some(expected)))
}

fn expected_prepared_payload<'a>(case_id: &str, payload: &'a str) -> Result<Cow<'a, str>, String> {
    if let Some((_, expected_raw, expected_prepared)) = PREPARED_PAYLOAD_OVERRIDES
        .iter()
        .find(|(override_case_id, _, _)| *override_case_id == case_id)
    {
        if payload != *expected_raw {
            return Err(format!(
                "authored preparation oracle no longer matches fixture payload: {case_id}"
            ));
        }
        return Ok(Cow::Borrowed(expected_prepared));
    }

    let has_unreviewed_ambiguous_control = payload.split("\r\n").any(|segment| {
        segment
            .chars()
            .any(|character| matches!(character, '\r' | '\u{000C}' | '\u{0085}'))
    });
    if has_unreviewed_ambiguous_control {
        return Err(format!(
            "targeted case needs an authored preparation oracle: {case_id}"
        ));
    }

    Ok(Cow::Borrowed(payload))
}

fn assert_preparation_preserves_fixture_boundaries(
    case_id: &str,
    case: &Value,
    prepared_source: &str,
) -> Result<(), String> {
    let facts = &case["portable_unicode_facts"];
    assert_eq!(
        prepared_source.encode_utf16().count(),
        required_usize(facts, "utf16_units")?,
        "product preparation changed the UTF-16 cardinality for {case_id}"
    );
    let graphemes = prepared_source.graphemes(true).collect::<Vec<_>>();
    assert_eq!(
        graphemes.len(),
        required_usize(facts, "graphemes")?,
        "product preparation changed the EGC cardinality for {case_id}"
    );
    assert_eq!(
        egc_end_utf16_offsets(&graphemes),
        required_usize_array(case, "egc_end_utf16_offsets")?,
        "product preparation changed EGC boundaries for {case_id}"
    );
    Ok(())
}

fn first_oversized_grapheme_utf16_units(text: &str) -> Option<usize> {
    text.graphemes(true)
        .map(|grapheme| grapheme.encode_utf16().count())
        .find(|utf16_units| *utf16_units > CHATBOX_MAX_UTF16_UNITS)
}

fn newest_oversized_grapheme(text: &str) -> Option<(usize, usize)> {
    text.grapheme_indices(true)
        .fold(None, |newest, (start, grapheme)| {
            let utf16_units = grapheme.encode_utf16().count();
            if utf16_units > CHATBOX_MAX_UTF16_UNITS {
                Some((start + grapheme.len(), utf16_units))
            } else {
                newest
            }
        })
}

fn egc_end_utf16_offsets(graphemes: &[&str]) -> Vec<usize> {
    graphemes
        .iter()
        .scan(0, |offset, grapheme| {
            *offset += grapheme.encode_utf16().count();
            Some(*offset)
        })
        .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn assert_no_forbidden_expectation_fields(value: &Value) {
    match value {
        Value::Array(values) => {
            for value in values {
                assert_no_forbidden_expectation_fields(value);
            }
        }
        Value::Object(object) => {
            for (field, value) in object {
                assert!(
                    !FORBIDDEN_GENERATED_EXPECTATION_FIELDS.contains(&field.as_str()),
                    "portable fixture contains non-portable field: {field}"
                );
                assert_no_forbidden_expectation_fields(value);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}
