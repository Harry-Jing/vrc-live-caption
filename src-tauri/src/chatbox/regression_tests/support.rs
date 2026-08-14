use super::super::layout::CHATBOX_MAX_UTF16_UNITS;
use serde_json::Value;
use sha2::{Digest, Sha256};
use unicode_segmentation::UnicodeSegmentation;

pub(super) const CHATBOX_REGRESSION_CORPUS_JSON: &str =
    include_str!("../../../testdata/chatbox/layout-cases-v1.json");
pub(super) const VRCHAT_CLIENT_OBSERVATIONS_JSON: &str = include_str!(
    "../../../testdata/chatbox/vrchat-client-observations-2026.3.1-1885-81193b80fa-v1.json"
);

// These are product-policy expectations, not observations copied from a
// VRChat build. Keeping the small set explicit makes a newly targeted control
// character fail closed until its intended preparation is reviewed.
pub(super) const PREPARATION_POLICY_EXPECTATIONS: [(&str, &str, &str); 3] = [
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

pub(super) fn require_object_fields(
    value: &Value,
    context: &str,
    required: &[&str],
    optional: &[&str],
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{context} must be an object."))?;

    for field in required {
        if !object.contains_key(*field) {
            return Err(format!("{context} is missing required field {field}."));
        }
    }
    for field in object.keys() {
        if !required.contains(&field.as_str()) && !optional.contains(&field.as_str()) {
            return Err(format!("{context} contains unexpected field {field}."));
        }
    }

    Ok(())
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
