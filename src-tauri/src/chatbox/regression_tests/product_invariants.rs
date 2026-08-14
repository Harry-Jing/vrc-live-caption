use super::super::layout::{
    CHATBOX_MAX_UTF16_UNITS, ChatboxLayoutError, predict_layout, prepare_completed_pages,
    prepare_live_viewport, prepare_single_message,
};
use super::support::{
    CHATBOX_REGRESSION_CORPUS_JSON, PREPARATION_POLICY_EXPECTATIONS, egc_end_utf16_offsets,
    first_oversized_grapheme_utf16_units, has_test_target, required_string, required_usize,
    required_usize_array,
};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::HashSet;
use unicode_segmentation::UnicodeSegmentation;

const EXPECTED_COMPLETED_TARGET_COUNT: usize = 98;
const EXPECTED_LIVE_TARGET_COUNT: usize = 96;
const EXPECTED_LAYOUT_TARGET_COUNT: usize = 169;

#[test]
fn completed_targets_form_lossless_standalone_prepared_pages() -> Result<(), String> {
    let corpus = serde_json::from_str::<Value>(CHATBOX_REGRESSION_CORPUS_JSON)
        .map_err(|error| error.to_string())?;
    let cases = corpus["cases"]
        .as_array()
        .ok_or("Chatbox corpus cases must be an array.")?;
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

        let result = prepare_completed_pages(payload);
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
            format!("Completed rejected representable corpus case {case_id}: {error:?}")
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
        "Chatbox regression corpus lost oversized EGC coverage"
    );
    Ok(())
}

#[test]
fn live_targets_form_bounded_newest_standalone_suffixes() -> Result<(), String> {
    let corpus = serde_json::from_str::<Value>(CHATBOX_REGRESSION_CORPUS_JSON)
        .map_err(|error| error.to_string())?;
    let cases = corpus["cases"]
        .as_array()
        .ok_or("Chatbox corpus cases must be an array.")?;
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
        let result = prepare_live_viewport(payload);
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
        "Chatbox regression corpus lost oversized EGC coverage"
    );
    assert_eq!(
        oversized_error_count, 1,
        "Chatbox regression corpus must distinguish old from newest oversized EGCs"
    );
    Ok(())
}

#[test]
fn layout_targets_have_predictions_without_vrchat_client_observation_expectations()
-> Result<(), String> {
    let corpus = serde_json::from_str::<Value>(CHATBOX_REGRESSION_CORPUS_JSON)
        .map_err(|error| error.to_string())?;
    let cases = corpus["cases"]
        .as_array()
        .ok_or("Chatbox corpus cases must be an array.")?;
    let mut target_count = 0;

    for case in cases {
        if !has_test_target(case, "layout")? {
            continue;
        }
        target_count += 1;

        let case_id = required_string(case, "case_id")?;
        let payload = required_string(case, "payload")?;
        let prepared_source = expected_prepared_payload(case_id, payload)?;
        let oversized = first_oversized_grapheme_utf16_units(prepared_source.as_ref());
        let first = predict_layout(payload);
        let second = predict_layout(payload);
        assert_eq!(
            first, second,
            "layout prediction was not deterministic: {case_id}"
        );

        if let Some(utf16_units) = oversized {
            assert_eq!(
                first,
                Err(ChatboxLayoutError::GraphemeExceedsInputBudget { utf16_units }),
                "layout prediction returned the wrong oversized-EGC error: {case_id}"
            );
            continue;
        }

        let prediction = first.map_err(|error| {
            format!("layout prediction rejected representable case {case_id}: {error:?}")
        })?;
        assert!(
            prediction.visible_line_count() <= 9,
            "layout prediction exceeded the visible-line cap: {case_id}"
        );
        assert!(
            prediction.logical_line_count() >= prediction.visible_line_count(),
            "layout prediction reported more visible than logical lines: {case_id}"
        );
        assert_eq!(
            prediction.is_clipped(),
            prediction.logical_line_count() > prediction.visible_line_count(),
            "layout prediction clipping flag was internally inconsistent: {case_id}"
        );

        let egc_ends = prepared_source
            .graphemes(true)
            .scan(0, |offset, grapheme| {
                *offset += grapheme.encode_utf16().count();
                Some(*offset)
            })
            .collect::<HashSet<_>>();
        assert_prediction_breaks_are_safe(
            case_id,
            prediction.soft_break_utf16_offsets(),
            &egc_ends,
        )?;
        assert_prediction_breaks_are_safe(
            case_id,
            prediction.explicit_break_utf16_offsets(),
            &egc_ends,
        )?;
        assert!(
            prediction
                .soft_break_utf16_offsets()
                .iter()
                .all(|offset| !prediction.explicit_break_utf16_offsets().contains(offset)),
            "layout prediction classified one break as both soft and explicit: {case_id}"
        );
    }

    assert_eq!(target_count, EXPECTED_LAYOUT_TARGET_COUNT);
    Ok(())
}

fn assert_prediction_breaks_are_safe(
    case_id: &str,
    offsets: &[usize],
    egc_ends: &HashSet<usize>,
) -> Result<(), String> {
    if !offsets.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(format!(
            "layout prediction break offsets were not strictly increasing: {case_id}"
        ));
    }
    if !offsets.iter().all(|offset| egc_ends.contains(offset)) {
        return Err(format!(
            "layout prediction break offset split an EGC or exceeded the payload: {case_id}"
        ));
    }
    Ok(())
}

fn expected_prepared_payload<'a>(case_id: &str, payload: &'a str) -> Result<Cow<'a, str>, String> {
    if let Some((_, expected_raw, expected_prepared)) = PREPARATION_POLICY_EXPECTATIONS
        .iter()
        .find(|(override_case_id, _, _)| *override_case_id == case_id)
    {
        if payload != *expected_raw {
            return Err(format!(
                "authored preparation oracle no longer matches corpus payload: {case_id}"
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
    let facts = &case["unicode_facts"];
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
