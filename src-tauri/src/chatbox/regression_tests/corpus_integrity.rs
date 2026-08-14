use super::support::{
    EXPECTED_MANIFEST_SHA256, EXPECTED_SOURCE_SHA256, PORTABLE_CORPUS_JSON,
    assert_no_forbidden_expectation_fields, egc_end_utf16_offsets, required_string, required_usize,
    required_usize_array, sha256_hex,
};
use serde_json::Value;
use std::collections::HashSet;
use unicode_segmentation::UnicodeSegmentation;

const EXPECTED_CASE_COUNT: usize = 178;
const ALLOWED_TEST_TARGETS: [&str; 4] = [
    "completed-pagination",
    "layout",
    "live-window",
    "sender-policy",
];

#[test]
fn portable_chatbox_corpus_has_stable_identity_and_unicode_facts()
-> Result<(), Box<dyn std::error::Error>> {
    let corpus = serde_json::from_str::<Value>(PORTABLE_CORPUS_JSON)?;
    let cases = corpus["cases"]
        .as_array()
        .ok_or("Chatbox corpus cases must be an array.")?;

    assert_eq!(corpus["fixture_schema_version"], 1);
    assert_eq!(corpus["case_count"], EXPECTED_CASE_COUNT);
    assert_eq!(cases.len(), EXPECTED_CASE_COUNT);
    assert_eq!(corpus["source"]["corpus_id"], "vrchat-chatbox-canonical");
    assert_eq!(corpus["source"]["source_sha256"], EXPECTED_SOURCE_SHA256);
    assert_eq!(
        corpus["source"]["manifest_sha256"],
        EXPECTED_MANIFEST_SHA256
    );
    assert_eq!(corpus["source"]["unicode_profile"]["version"], "17.0.0");
    assert_no_forbidden_expectation_fields(&corpus);

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

    assert!(!PORTABLE_CORPUS_JSON.contains("C:\\\\Users\\\\Harry\\\\"));
    assert!(!PORTABLE_CORPUS_JSON.contains("vrc-chatbox-layout-lab"));
    assert!(!PORTABLE_CORPUS_JSON.contains("captures/"));
    assert!(!PORTABLE_CORPUS_JSON.contains("captures\\\\"));
    Ok(())
}
