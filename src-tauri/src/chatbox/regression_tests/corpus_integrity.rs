use super::support::{
    CHATBOX_REGRESSION_CORPUS_JSON, egc_end_utf16_offsets, require_object_fields, required_string,
    required_usize, required_usize_array, sha256_hex,
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
fn chatbox_regression_corpus_has_stable_identity_and_unicode_facts()
-> Result<(), Box<dyn std::error::Error>> {
    let corpus = serde_json::from_str::<Value>(CHATBOX_REGRESSION_CORPUS_JSON)?;
    let cases = corpus["cases"]
        .as_array()
        .ok_or("Chatbox corpus cases must be an array.")?;

    require_object_fields(
        &corpus,
        "Chatbox corpus",
        &[
            "case_count",
            "cases",
            "description",
            "fixture_schema_version",
        ],
        &[],
    )?;

    assert_eq!(corpus["fixture_schema_version"], 1);
    assert_eq!(corpus["case_count"], EXPECTED_CASE_COUNT);
    assert_eq!(cases.len(), EXPECTED_CASE_COUNT);

    let mut case_ids = HashSet::new();
    let mut payloads = HashSet::new();
    for case in cases {
        let case_id = required_string(case, "case_id")?;
        let payload = required_string(case, "payload")?;
        let facts = &case["unicode_facts"];

        require_object_fields(
            case,
            &format!("Chatbox corpus case {case_id}"),
            &[
                "base_direction",
                "case_id",
                "egc_end_utf16_offsets",
                "egc_safe_prefix_144",
                "features",
                "intent",
                "language_tags",
                "payload",
                "payload_sha256",
                "unicode_facts",
                "scripts",
                "test_targets",
            ],
            &["relation"],
        )?;
        require_object_fields(
            facts,
            &format!("Unicode facts for {case_id}"),
            &[
                "budget_class",
                "code_points",
                "egc_safe_prefix_144",
                "graphemes",
                "line_breaks",
                "normalization",
                "utf16_units",
                "utf8_bytes",
            ],
            &[],
        )?;
        require_object_fields(
            &facts["egc_safe_prefix_144"],
            &format!("safe-prefix facts for {case_id}"),
            &["equals_payload", "graphemes", "utf16_units"],
            &[],
        )?;
        require_object_fields(
            &facts["line_breaks"],
            &format!("line-break facts for {case_id}"),
            &[
                "cr",
                "crlf",
                "lf",
                "line_separator",
                "nel",
                "paragraph_separator",
            ],
            &[],
        )?;
        require_object_fields(
            &facts["normalization"],
            &format!("normalization facts for {case_id}"),
            &["is_nfc", "is_nfd"],
            &[],
        )?;
        if case.get("relation").is_some() {
            require_object_fields(
                &case["relation"],
                &format!("relation for {case_id}"),
                &["group", "member"],
                &[],
            )?;
        }

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

    Ok(())
}
