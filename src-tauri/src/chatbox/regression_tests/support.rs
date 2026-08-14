use super::super::layout::CHATBOX_MAX_UTF16_UNITS;
use serde_json::Value;
use sha2::{Digest, Sha256};
use unicode_segmentation::UnicodeSegmentation;

pub(super) const FIXTURE: &str = include_str!("../../../testdata/chatbox/layout-cases-v1.json");
pub(super) const RUNTIME_OBSERVATIONS: &str =
    include_str!("../../../testdata/chatbox/runtime-observations-2026.3.1-1885-81193b80fa-v1.json");
pub(super) const EXPECTED_SOURCE_SHA256: &str =
    "f4899d95d0a2fac74a96423608cd4d9b88fa3afe28737c747356fcd3d4190731";
pub(super) const EXPECTED_MANIFEST_SHA256: &str =
    "fcbad159f2fb9ea6f0b9bd9fab33373c913d48e7b3213913bb73c8a3ac68e7d8";
pub(super) const FORBIDDEN_GENERATED_EXPECTATION_FIELDS: [&str; 8] = [
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
pub(super) const PREPARED_PAYLOAD_OVERRIDES: [(&str, &str, &str); 3] = [
    ("LINES-CR-BASIC", "alpha\rbeta", "alpha beta"),
    ("LINES-NEL-BASIC", "alpha\u{0085}beta", "alpha beta"),
    (
        "LINES-NINE-CR",
        "one\rtwo\rthree\rfour\rfive\rsix\rseven\reight\rnine",
        "one two three four five six seven eight nine",
    ),
];

pub(super) fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value[field]
        .as_str()
        .ok_or_else(|| format!("{field} must be a string."))
}

pub(super) fn required_usize(value: &Value, field: &str) -> Result<usize, String> {
    value[field]
        .as_u64()
        .and_then(|number| usize::try_from(number).ok())
        .ok_or_else(|| format!("{field} must be a non-negative integer."))
}

pub(super) fn required_usize_array(value: &Value, field: &str) -> Result<Vec<usize>, String> {
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

pub(super) fn has_test_target(case: &Value, expected: &str) -> Result<bool, String> {
    let case_id = required_string(case, "case_id")?;
    let targets = case["test_targets"]
        .as_array()
        .ok_or_else(|| format!("test_targets must be an array: {case_id}"))?;

    Ok(targets
        .iter()
        .any(|target| target.as_str() == Some(expected)))
}

pub(super) fn first_oversized_grapheme_utf16_units(text: &str) -> Option<usize> {
    text.graphemes(true)
        .map(|grapheme| grapheme.encode_utf16().count())
        .find(|utf16_units| *utf16_units > CHATBOX_MAX_UTF16_UNITS)
}

pub(super) fn egc_end_utf16_offsets(graphemes: &[&str]) -> Vec<usize> {
    graphemes
        .iter()
        .scan(0, |offset, grapheme| {
            *offset += grapheme.encode_utf16().count();
            Some(*offset)
        })
        .collect()
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn assert_no_forbidden_expectation_fields(value: &Value) {
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
