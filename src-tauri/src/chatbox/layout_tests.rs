use super::model::{
    MAX_GRAPHEME_ADVANCE_UNITS, POSITIVE_KERNING_PAIRS, fits_chatbox_width, grapheme_advance_units,
    positive_kerning_adjustment,
};
use super::{
    CHATBOX_MAX_UTF16_UNITS, ChatboxLayoutError, PreparedChatboxText, is_break_space_grapheme,
    predict_layout, prepare_completed_pages, prepare_live_viewport, prepare_single_message,
};
use proptest::prelude::*;
use std::collections::HashSet;
use unicode_segmentation::UnicodeSegmentation;

const LAYOUT_PROPERTY_CASES: u32 = 64;
const MAX_GENERATED_GRAPHEME_ATOMS: usize = 160;

const CJK_135: &str = concat!(
    "中中中中中中中中中中中中中中中",
    "中中中中中中中中中中中中中中中",
    "中中中中中中中中中中中中中中中",
    "中中中中中中中中中中中中中中中",
    "中中中中中中中中中中中中中中中",
    "中中中中中中中中中中中中中中中",
    "中中中中中中中中中中中中中中中",
    "中中中中中中中中中中中中中中中",
    "中中中中中中中中中中中中中中中",
);
const CJK_134: &str = concat!(
    "中中中中中中中中中中中中中中中",
    "中中中中中中中中中中中中中中中",
    "中中中中中中中中中中中中中中中",
    "中中中中中中中中中中中中中中中",
    "中中中中中中中中中中中中中中中",
    "中中中中中中中中中中中中中中中",
    "中中中中中中中中中中中中中中中",
    "中中中中中中中中中中中中中中中",
    "中中中中中中中中中中中中中中",
);
const X_144: &str = concat!(
    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
);
const X_143: &str = concat!(
    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
);
const X_134: &str = concat!(
    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
    "xxxxxxxxxxxxxxxxxxxxxxxxxx",
);
const WORDS_28_WITH_SPACE: &str = concat!(
    "word word word word word word word word word word ",
    "word word word word word word word word word word ",
    "word word word word word word word word ",
);
const WORDS_30: &str = concat!(
    "word word word word word word word word word word ",
    "word word word word word word word word word word ",
    "word word word word word word word word word word",
);
const HELLO_20_WITH_SPACE: &str = concat!(
    "hello hello hello hello hello hello hello hello hello hello ",
    "hello hello hello hello hello hello hello hello hello hello ",
);
const X_30: &str = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";
const MIXED_9_LINES: &str = concat!(
    "中中中中中xxxxxxxxxx\n",
    "中中中中中xxxxxxxxxx\n",
    "中中中中中xxxxxxxxxx\n",
    "中中中中中xxxxxxxxxx\n",
    "中中中中中xxxxxxxxxx\n",
    "中中中中中xxxxxxxxxx\n",
    "中中中中中xxxxxxxxxx\n",
    "中中中中中xxxxxxxxxx\n",
    "中中中中中xxxxxxxxxx",
);
const MIXED_WRAP_UNIT: &str = "中中中中中中中中中中中中中中W";
const MIXED_9_WRAPPED_LINES: &str = concat!(
    "中中中中中中中中中中中中中中W",
    "中中中中中中中中中中中中中中W",
    "中中中中中中中中中中中中中中W",
    "中中中中中中中中中中中中中中W",
    "中中中中中中中中中中中中中中W",
    "中中中中中中中中中中中中中中W",
    "中中中中中中中中中中中中中中W",
    "中中中中中中中中中中中中中中W",
    "中中中中中中中中中中中中中中W",
);
const X_140: &str = concat!(
    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
);
const EMOJI_9: &str = "😀😀😀😀😀😀😀😀😀";
const TONED_EMOJI_9: &str = "👍🏽👍🏽👍🏽👍🏽👍🏽👍🏽👍🏽👍🏽👍🏽";
const STANDALONE_SKIN_TONE_9: &str = "🏽 🏽 🏽 🏽 🏽 🏽 🏽 🏽 🏽";
const ARABIC_9: &str = "ممممممممم";
const WIDE_PUNCTUATION_9: &str = "⸻⸻⸻⸻⸻⸻⸻⸻⸻";

fn prepared_texts(pages: &[PreparedChatboxText]) -> Vec<&str> {
    pages.iter().map(PreparedChatboxText::as_str).collect()
}

fn concat_prepared(pages: &[PreparedChatboxText]) -> String {
    pages.iter().map(PreparedChatboxText::as_str).collect()
}

fn require_live_view(input: &str) -> Result<PreparedChatboxText, String> {
    prepare_live_viewport(input)
        .map_err(|error| format!("{error:?}"))?
        .ok_or_else(|| "nonempty Live input produced no viewport".to_owned())
}

fn representative_grapheme_text() -> impl Strategy<Value = String> {
    let representable_atom = prop::sample::select(vec![
        "a",
        "W",
        "中",
        "語",
        "e\u{301}",
        "#\u{301}",
        "👨‍👩‍👧‍👦",
        "🧑‍💻",
        "👍🏽",
        "\n",
        "\r\n",
        "\u{000B}",
        "\u{2028}",
        "\u{2029}",
        " ",
        " \u{FE0F}",
        "「",
        "。",
        "’",
        "—",
        "(",
        ")",
    ])
    .prop_map(str::to_owned);
    let oversized_grapheme = format!("e{}", "\u{301}".repeat(CHATBOX_MAX_UTF16_UNITS));
    // Keep error inputs in the mix without crowding out successful Completed
    // pagination, whose lossless partition invariants are the primary target.
    let atom = prop_oneof![
        511 => representable_atom,
        1 => Just(oversized_grapheme),
    ];

    prop::collection::vec(atom, 0..=MAX_GENERATED_GRAPHEME_ATOMS).prop_map(|atoms| atoms.concat())
}

fn prepared_control_policy_text() -> impl Strategy<Value = (String, String)> {
    let atom = prop::sample::select(vec![
        ("x", "x"),
        ("中", "中"),
        ("\r", " "),
        ("\u{0085}", " "),
        ("\u{000C}", " "),
        ("\r\n", "\r\n"),
        ("\r\r\n", " \r\n"),
        ("\u{000B}", "\u{000B}"),
        ("\u{2028}", "\u{2028}"),
        ("\u{2029}", "\u{2029}"),
        ("\u{0301}", "\u{0301}"),
        ("e\u{0301}", "e\u{0301}"),
    ]);

    prop::collection::vec(atom, 0..=MAX_GENERATED_GRAPHEME_ATOMS).prop_map(|atoms| {
        let raw = atoms
            .iter()
            .map(|(raw, _)| *raw)
            .collect::<Vec<_>>()
            .concat();
        let expected = atoms
            .iter()
            .map(|(_, expected)| *expected)
            .collect::<Vec<_>>()
            .concat();
        (raw, expected)
    })
}

#[test]
fn layout_prediction_reports_the_verified_ascii_soft_wrap_boundary() -> Result<(), String> {
    let one_line = predict_layout(&"x".repeat(29)).map_err(|error| format!("{error:?}"))?;
    let two_lines = predict_layout(&"x".repeat(30)).map_err(|error| format!("{error:?}"))?;

    assert_eq!(one_line.logical_line_count(), 1);
    assert_eq!(one_line.visible_line_count(), 1);
    assert!(one_line.soft_break_utf16_offsets().is_empty());
    assert!(one_line.explicit_break_utf16_offsets().is_empty());
    assert!(!one_line.is_clipped());

    assert_eq!(two_lines.logical_line_count(), 2);
    assert_eq!(two_lines.visible_line_count(), 2);
    assert_eq!(two_lines.soft_break_utf16_offsets(), &[29]);
    assert!(two_lines.explicit_break_utf16_offsets().is_empty());
    assert!(!two_lines.is_clipped());
    Ok(())
}

#[test]
fn positive_kerning_can_push_an_otherwise_fitting_line_past_the_width_limit() -> Result<(), String>
{
    // Both strings have 15,456 units of unkerned advance. The extracted
    // primary-font GPOS pair for `¿J` adds the maximum positive adjustment
    // (+100); reversing only that pair leaves the same base advances.
    let prefix = format!("{}{}", "x".repeat(21), " ".repeat(14));
    let without_positive_pair =
        predict_layout(&format!("{prefix}J¿")).map_err(|error| format!("{error:?}"))?;
    let with_positive_pair =
        predict_layout(&format!("{prefix}¿J")).map_err(|error| format!("{error:?}"))?;

    assert_eq!(without_positive_pair.logical_line_count(), 1);
    assert_eq!(with_positive_pair.logical_line_count(), 2);
    assert_eq!(with_positive_pair.soft_break_utf16_offsets(), &[35]);
    Ok(())
}

#[test]
fn positive_kerning_table_matches_the_hash_pinned_font_extraction() {
    assert_eq!(POSITIVE_KERNING_PAIRS.len(), 105);
    assert!(
        POSITIVE_KERNING_PAIRS
            .windows(2)
            .all(|pairs| (pairs[0].0, pairs[0].1) < (pairs[1].0, pairs[1].1))
    );
    assert!(
        POSITIVE_KERNING_PAIRS
            .iter()
            .all(|&(_, _, adjustment)| adjustment > 0)
    );
    assert_eq!(
        POSITIVE_KERNING_PAIRS
            .iter()
            .map(|&(_, _, adjustment)| adjustment)
            .max(),
        Some(100)
    );
    assert_eq!(positive_kerning_adjustment('T', 'T'), 20);
    assert_eq!(positive_kerning_adjustment('\u{00BF}', 'J'), 100);
    assert_eq!(positive_kerning_adjustment('A', 'V'), 0);
}

#[test]
fn positive_kerning_context_resets_at_line_boundaries() -> Result<(), String> {
    // The first line's final `¿J` pair triggers a soft wrap before J. The J-led
    // second line has 15,458 base units: it fits only when the pair from the
    // preceding line is not carried across the boundary.
    let second_line = format!("J{}{}", "x".repeat(17), "i".repeat(24));
    let soft = predict_layout(&format!(
        "{}{}¿{second_line}",
        "x".repeat(11),
        "z".repeat(19)
    ))
    .map_err(|error| format!("{error:?}"))?;
    let explicit =
        predict_layout(&format!("¿\n{second_line}")).map_err(|error| format!("{error:?}"))?;

    assert_eq!(soft.logical_line_count(), 2);
    assert_eq!(soft.soft_break_utf16_offsets(), &[31]);
    assert_eq!(explicit.logical_line_count(), 2);
    assert_eq!(explicit.explicit_break_utf16_offsets(), &[2]);
    assert!(explicit.soft_break_utf16_offsets().is_empty());
    Ok(())
}

#[test]
fn completed_pages_remain_safe_when_positive_kerning_creates_a_tenth_line() -> Result<(), String> {
    // Eight conservative full-line graphemes put the final unbroken run on
    // line nine. Its base advances fit, but the +100 `¿J` pair creates a real
    // tenth line while the whole source remains well below 144 UTF-16 units.
    let prefix = format!("{}{}{}", "😀".repeat(8), "x".repeat(11), "z".repeat(19));
    let fitting = predict_layout(&format!("{prefix}J¿")).map_err(|error| format!("{error:?}"))?;
    let input = format!("{prefix}¿J");
    let widened = predict_layout(&input).map_err(|error| format!("{error:?}"))?;

    assert_eq!(fitting.logical_line_count(), 9);
    assert_eq!(widened.logical_line_count(), 10);
    assert!(widened.is_clipped());

    let pages = prepare_completed_pages(&input).map_err(|error| format!("{error:?}"))?;

    assert_eq!(pages.len(), 2);
    assert_eq!(concat_prepared(&pages), input);
    for page in pages {
        let prediction = predict_layout(page.as_str()).map_err(|error| format!("{error:?}"))?;
        assert!(prediction.logical_line_count() <= 9);
        assert!(!prediction.is_clipped());
    }
    Ok(())
}

#[test]
fn layout_prediction_uses_prepared_utf16_offsets_for_verified_separators() -> Result<(), String> {
    let source = "😀\na\r\nb\u{000B}c\u{2028}d\u{2029}e\rf\u{0085}g\u{000C}h";
    let prediction = predict_layout(source).map_err(|error| format!("{error:?}"))?;

    assert_eq!(prediction.logical_line_count(), 6);
    assert_eq!(prediction.visible_line_count(), 6);
    assert!(prediction.soft_break_utf16_offsets().is_empty());
    assert_eq!(
        prediction.explicit_break_utf16_offsets(),
        &[3, 6, 8, 10, 12]
    );
    assert!(!prediction.is_clipped());
    Ok(())
}

#[test]
fn layout_prediction_distinguishes_logical_lines_from_the_nine_visible_lines() -> Result<(), String>
{
    let nine_lines =
        predict_layout("1\n2\n3\n4\n5\n6\n7\n8\n9").map_err(|error| format!("{error:?}"))?;
    let ten_lines =
        predict_layout("1\n2\n3\n4\n5\n6\n7\n8\n9\n0").map_err(|error| format!("{error:?}"))?;

    assert_eq!(nine_lines.logical_line_count(), 9);
    assert_eq!(nine_lines.visible_line_count(), 9);
    assert_eq!(
        nine_lines.explicit_break_utf16_offsets(),
        &[2, 4, 6, 8, 10, 12, 14, 16]
    );
    assert!(!nine_lines.is_clipped());

    assert_eq!(ten_lines.logical_line_count(), 10);
    assert_eq!(ten_lines.visible_line_count(), 9);
    assert_eq!(
        ten_lines.explicit_break_utf16_offsets(),
        &[2, 4, 6, 8, 10, 12, 14, 16, 18]
    );
    assert!(ten_lines.is_clipped());

    let trailing_separator = predict_layout("alpha\n").map_err(|error| format!("{error:?}"))?;
    assert_eq!(trailing_separator.logical_line_count(), 1);
    assert!(trailing_separator.explicit_break_utf16_offsets().is_empty());
    Ok(())
}

#[test]
fn terminal_separator_stays_with_nine_lines_but_a_real_tenth_line_does_not() -> Result<(), String> {
    let nine_lines = "1\n2\n3\n4\n5\n6\n7\n8\n9";
    let terminal_separator = format!("{nine_lines}\n");
    let terminal_pages =
        prepare_completed_pages(&terminal_separator).map_err(|error| format!("{error:?}"))?;
    assert_eq!(prepared_texts(&terminal_pages), vec![terminal_separator]);

    let tenth_line = format!("{nine_lines}\n0");
    let tenth_line_pages =
        prepare_completed_pages(&tenth_line).map_err(|error| format!("{error:?}"))?;
    assert_eq!(prepared_texts(&tenth_line_pages), vec![nine_lines, "\n0"]);
    Ok(())
}

#[test]
fn layout_prediction_applies_tmp_kinsoku_to_soft_break_offsets() -> Result<(), String> {
    let opening = predict_layout(&format!("{}「中", "中".repeat(14)))
        .map_err(|error| format!("{error:?}"))?;
    let closing =
        predict_layout(&format!("{}。", "中".repeat(15))).map_err(|error| format!("{error:?}"))?;

    assert_eq!(opening.logical_line_count(), 2);
    assert_eq!(opening.soft_break_utf16_offsets(), &[14]);
    assert!(opening.explicit_break_utf16_offsets().is_empty());

    assert_eq!(closing.logical_line_count(), 2);
    assert_eq!(closing.soft_break_utf16_offsets(), &[14]);
    assert!(closing.explicit_break_utf16_offsets().is_empty());
    Ok(())
}

#[test]
fn single_view_prepares_exactly_one_independently_safe_message() -> Result<(), String> {
    let prepared = prepare_single_message("first line\n  second  line")
        .map_err(|error| format!("{error:?}"))?
        .ok_or("nonempty single view was omitted")?;

    assert_eq!(prepared.as_str(), "first line\n  second  line");
    assert_eq!(prepare_single_message(""), Ok(None));
    assert_eq!(
        prepare_single_message(&format!("{X_144}x")),
        Err(ChatboxLayoutError::RequiresPagination { page_count: 2 })
    );
    Ok(())
}

#[test]
fn preparation_replaces_ambiguous_controls_and_preserves_verified_text() -> Result<(), String> {
    let source = "\rA\r\nB\u{0085}C\u{000C}D\u{000B}E\u{2028}F\u{2029}G\nCafé|Cafe\u{0301}";
    let expected = " A\r\nB C D\u{000B}E\u{2028}F\u{2029}G\nCafé|Cafe\u{0301}";

    let prepared = prepare_single_message(source)
        .map_err(|error| format!("{error:?}"))?
        .ok_or("nonempty prepared message was omitted")?;

    assert_eq!(prepared.as_str(), expected);

    let adjacent = prepare_single_message("\r\r\n\u{0085}\u{000C}")
        .map_err(|error| format!("{error:?}"))?
        .ok_or("nonempty adjacent controls were omitted")?;
    assert_eq!(adjacent.as_str(), " \r\n  ");
    Ok(())
}

#[test]
fn preparation_replaces_nel_before_line_measurement() -> Result<(), String> {
    let source = concat!(
        "a\u{0085}b\n",
        "c\n",
        "d\n",
        "e\n",
        "f\n",
        "g\n",
        "h\n",
        "i\n",
        "j",
    );
    let expected = "a b\nc\nd\ne\nf\ng\nh\ni\nj";

    let prepared = prepare_single_message(source)
        .map_err(|error| format!("{error:?}"))?
        .ok_or("nonempty prepared message was omitted")?;

    assert_eq!(prepared.as_str(), expected);
    Ok(())
}

#[test]
fn preparation_precedes_the_utf16_pagination_boundary() -> Result<(), String> {
    let within_source = format!("{}\u{0085}x", "x".repeat(142));
    let within_expected = format!("{} x", "x".repeat(142));
    let source = format!("{}\u{0085}x", "x".repeat(143));
    let expected = format!("{} x", "x".repeat(143));

    let within = prepare_single_message(&within_source)
        .map_err(|error| format!("{error:?}"))?
        .ok_or("144-unit prepared message was omitted")?;
    let pages = prepare_completed_pages(&source).map_err(|error| format!("{error:?}"))?;

    assert_eq!(within.as_str(), within_expected);
    assert_eq!(within.as_str().encode_utf16().count(), 144);
    assert_eq!(pages.len(), 2);
    assert_eq!(concat_prepared(&pages), expected);
    assert!(
        pages
            .iter()
            .all(|page| page.as_str().encode_utf16().count() <= CHATBOX_MAX_UTF16_UNITS)
    );

    let changed_grapheme_source = format!("{X_143}\u{0085}\u{0301}");
    let changed_grapheme_pages =
        prepare_completed_pages(&changed_grapheme_source).map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        prepared_texts(&changed_grapheme_pages),
        vec![X_143, " \u{0301}"]
    );
    Ok(())
}

#[test]
fn completed_preparation_preserves_verified_breaks_and_replaces_ambiguous_controls()
-> Result<(), String> {
    let cases = [
        ("LF", "one\ntwo\n中", "one\ntwo\n中"),
        ("CRLF", "one\r\ntwo", "one\r\ntwo"),
        ("vertical tab", "one\u{000B}two", "one\u{000B}two"),
        ("bare CR", "one\rtwo", "one two"),
        ("NEL", "one\u{0085}two", "one two"),
        ("form feed", "one\u{000C}two", "one two"),
        (
            "Unicode line and paragraph separators",
            "甲\u{2028}乙\u{2029}丙",
            "甲\u{2028}乙\u{2029}丙",
        ),
        (
            "leading and trailing line breaks",
            "\n\nleading and trailing\n",
            "\n\nleading and trailing\n",
        ),
    ];

    for (name, input, expected) in cases {
        let pages = prepare_completed_pages(input).map_err(|error| format!("{error:?}"))?;
        assert_eq!(prepared_texts(&pages), vec![expected], "{name}");
    }

    Ok(())
}

#[test]
fn break_space_classification_keeps_visible_marks_in_layout() {
    let cases = [
        ("ASCII space", " ", true),
        ("space with variation selector", " \u{FE0F}", true),
        ("space with visible spacing mark", " \u{093E}", false),
        ("visible text", "x", false),
    ];

    for (name, grapheme, expected) in cases {
        assert_eq!(grapheme.graphemes(true).count(), 1, "{name}");
        assert_eq!(is_break_space_grapheme(grapheme), expected, "{name}");
    }
}

#[test]
fn completed_layout_paginates_continuous_chinese_after_nine_lines() -> Result<(), String> {
    let input = format!("{CJK_135}中");
    let pages = prepare_completed_pages(&input).map_err(|error| format!("{error:?}"))?;

    assert_eq!(prepared_texts(&pages), vec![CJK_135, "中"]);

    Ok(())
}

#[test]
fn measured_glyph_advances_match_vrchat_capacity_anchors() {
    let cases = [
        ("lowercase x", "x", 29),
        ("uppercase X", "X", 26),
        ("digit one", "1", 27),
        ("lowercase m", "m", 16),
        ("lowercase w", "w", 19),
        ("uppercase W", "W", 16),
        ("digit zero", "0", 27),
        ("ASCII period", ".", 58),
        ("ASCII colon", ":", 58),
        ("accented Latin letter", "é", 27),
        ("curly apostrophe", "’", 88),
        ("curly opening quote", "“", 43),
        ("em dash", "—", 15),
        ("CJK ideograph", "中", 15),
        ("full-width comma", "，", 15),
    ];

    for (name, grapheme, expected_capacity) in cases {
        let advance = grapheme_advance_units(grapheme);
        let mut actual_capacity = 0;
        while fits_chatbox_width(advance * (actual_capacity + 1)) {
            actual_capacity += 1;
        }

        assert_eq!(actual_capacity, expected_capacity, "{name}");
    }
}

#[test]
fn unshaped_variation_and_keycap_sequences_reserve_a_full_line() {
    let cases = [
        ("CJK ideographic variation sequence", "葛\u{E0100}"),
        ("emoji keycap sequence", "1\u{FE0F}\u{20E3}"),
        ("space with emoji variation selector", " \u{FE0F}"),
        ("isolated emoji variation selector", "\u{FE0F}"),
        ("isolated ideographic variation selector", "\u{E0100}"),
        ("isolated zero-width joiner", "\u{200D}"),
        ("isolated keycap mark", "\u{20E3}"),
        ("isolated emoji tag", "\u{E0067}"),
    ];

    for (name, grapheme) in cases {
        assert_eq!(grapheme.graphemes(true).count(), 1, "{name}");
        let advance = grapheme_advance_units(grapheme);
        assert_eq!(advance, MAX_GRAPHEME_ADVANCE_UNITS, "{name}");
        assert!(fits_chatbox_width(advance), "{name}");
    }
}

#[test]
fn whitespace_cannot_hide_an_ideographic_variation_selector_from_layout() -> Result<(), String> {
    // U+E0100 produced a visible missing-glyph marker on the measured build.
    // Unicode groups it with the preceding space, but TMP still processes the
    // selector scalar, so treating the whole EGC as an ordinary space can hide
    // enough width to cross the nine-line safety boundary.
    let input = format!("{}{}x", "😀".repeat(7), " \u{E0100}".repeat(43));
    assert_eq!(input.encode_utf16().count(), CHATBOX_MAX_UTF16_UNITS);

    let source_prediction = predict_layout(&input).map_err(|error| format!("{error:?}"))?;
    assert!(source_prediction.is_clipped());

    let pages = prepare_completed_pages(&input).map_err(|error| format!("{error:?}"))?;
    assert!(pages.len() > 1);
    assert_eq!(concat_prepared(&pages), input);
    for page in pages {
        let prediction = predict_layout(page.as_str()).map_err(|error| format!("{error:?}"))?;
        assert!(prediction.logical_line_count() <= 9);
        assert!(!prediction.is_clipped());
    }
    Ok(())
}

#[test]
fn selector_bearing_space_cannot_be_discarded_at_the_nine_line_boundary() -> Result<(), String> {
    let input = format!("{}{}", "😀".repeat(8), " \u{E0100}".repeat(2));

    let source_prediction = predict_layout(&input).map_err(|error| format!("{error:?}"))?;
    assert_eq!(source_prediction.logical_line_count(), 10);
    assert!(source_prediction.is_clipped());

    let pages = prepare_completed_pages(&input).map_err(|error| format!("{error:?}"))?;
    assert_eq!(pages.len(), 2);
    assert_eq!(concat_prepared(&pages), input);
    for page in pages {
        let prediction = predict_layout(page.as_str()).map_err(|error| format!("{error:?}"))?;
        assert!(prediction.logical_line_count() <= 9);
        assert!(!prediction.is_clipped());
    }
    Ok(())
}

#[test]
fn completed_layout_matches_language_and_length_fixtures() -> Result<(), String> {
    struct Case {
        name: &'static str,
        input: String,
        expected: Vec<String>,
    }

    let cases = [
        Case {
            name: "empty completed text",
            input: String::new(),
            expected: Vec::new(),
        },
        Case {
            name: "Chinese nine-line boundary",
            input: CJK_135.to_string(),
            expected: vec![CJK_135.to_string()],
        },
        Case {
            name: "English unbroken token",
            input: format!("{X_144}x"),
            expected: vec![X_144.to_string(), "x".to_string()],
        },
        Case {
            name: "English word boundary",
            input: WORDS_30.to_string(),
            expected: vec![WORDS_28_WITH_SPACE.to_string(), "word word".to_string()],
        },
        Case {
            name: "English avoids splitting a word across pages",
            input: format!("{HELLO_20_WITH_SPACE}{X_30}"),
            expected: vec![HELLO_20_WITH_SPACE.to_string(), X_30.to_string()],
        },
        Case {
            name: "mixed text with explicit tenth line",
            input: format!("{MIXED_9_LINES}\n尾"),
            expected: vec![MIXED_9_LINES.to_string(), "\n尾".to_string()],
        },
        Case {
            name: "mixed measured widths create nine soft-wrapped lines",
            input: format!("{MIXED_9_WRAPPED_LINES}{MIXED_WRAP_UNIT}"),
            expected: vec![
                MIXED_9_WRAPPED_LINES.to_string(),
                MIXED_WRAP_UNIT.to_string(),
            ],
        },
        Case {
            name: "three-page completed Chinese",
            input: format!("{CJK_135}{CJK_135}中"),
            expected: vec![CJK_135.to_string(), CJK_135.to_string(), "中".to_string()],
        },
        Case {
            name: "every page is safe when laid out without prior-page context",
            input: "न ा。 ा) ा#\u{301}न ा e\u{301}’😀-(🏽 \u{FE0F}مW\u{2028}🏽 ा".to_string(),
            expected: vec![
                "न ा。 ा) ा#\u{301}न ा".to_string(),
                " e\u{301}’😀-(🏽 \u{FE0F}مW\u{2028}".to_string(),
                "🏽 ा".to_string(),
            ],
        },
    ];

    for case in cases {
        let pages = prepare_completed_pages(&case.input).map_err(|error| format!("{error:?}"))?;
        let expected = case.expected.iter().map(String::as_str).collect::<Vec<_>>();
        assert_eq!(prepared_texts(&pages), expected, "{}", case.name);
        assert_eq!(
            concat_prepared(&pages),
            case.input,
            "{} lost content",
            case.name
        );
        assert!(
            pages
                .iter()
                .all(|page| page.as_str().encode_utf16().count() <= CHATBOX_MAX_UTF16_UNITS),
            "{} exceeded the UTF-16 budget",
            case.name
        );
        for page in &pages {
            let standalone_pages =
                prepare_completed_pages(page.as_str()).map_err(|error| format!("{error:?}"))?;
            assert_eq!(
                prepared_texts(&standalone_pages),
                vec![page.as_str()],
                "{} returned a page that is unsafe on its own",
                case.name
            );
        }
    }

    Ok(())
}

#[test]
fn completed_layout_keeps_tmp_punctuation_off_page_seams() -> Result<(), String> {
    let cases = [
        (
            "following punctuation",
            format!("{CJK_135}。"),
            vec![CJK_134.to_string(), "中。".to_string()],
        ),
        (
            "leading punctuation",
            format!("{CJK_134}「中"),
            vec![CJK_134.to_string(), "「中".to_string()],
        ),
        (
            "TMP-specific leading character",
            format!("{CJK_134}#中"),
            vec![CJK_134.to_string(), "#中".to_string()],
        ),
        (
            "following punctuation after a break-space",
            format!("{CJK_135} 。"),
            vec![CJK_134.to_string(), "中 。".to_string()],
        ),
        (
            "TMP-only following punctuation after a break-space",
            format!("{CJK_135}.. ’"),
            vec![CJK_134.to_string(), "中.. ’".to_string()],
        ),
        (
            "leading character before a break-space",
            format!("{CJK_134}# 中"),
            vec![CJK_134.to_string(), "# 中".to_string()],
        ),
        (
            "combining mark does not hide a leading character",
            format!("a {X_140}#\u{301}中"),
            vec!["a ".to_string(), format!("{X_140}#\u{301}中")],
        ),
        (
            "prepend format does not hide following punctuation",
            format!("{CJK_135}\u{600}。"),
            vec![CJK_134.to_string(), "中\u{600}。".to_string()],
        ),
        (
            "space modifier does not hide a leading character",
            format!("{CJK_134}# \u{FE0F}中"),
            vec![CJK_134.to_string(), "# \u{FE0F}中".to_string()],
        ),
    ];

    for (name, input, expected) in cases {
        let pages = prepare_completed_pages(&input).map_err(|error| format!("{error:?}"))?;
        let expected = expected.iter().map(String::as_str).collect::<Vec<_>>();
        assert_eq!(prepared_texts(&pages), expected, "{name}");
        assert_eq!(concat_prepared(&pages), input, "{name} lost content");
    }

    Ok(())
}

#[test]
fn completed_layout_never_splits_unicode_graphemes() -> Result<(), String> {
    let combining = "e\u{301}";
    let family = "👨‍👩‍👧‍👦";
    let toned_emoji = "👍🏽";
    let cases = [
        (
            "combining grapheme",
            format!("{X_143}{combining}"),
            vec![X_143.to_string(), combining.to_string()],
        ),
        (
            "ZWJ emoji family",
            format!("{X_134}{family}"),
            vec![X_134.to_string(), family.to_string()],
        ),
        (
            "non-BMP emoji",
            format!("{EMOJI_9}😀"),
            vec![EMOJI_9.to_string(), "😀".to_string()],
        ),
        (
            "emoji modifier sequence",
            format!("{TONED_EMOJI_9}{toned_emoji}"),
            vec![TONED_EMOJI_9.to_string(), toned_emoji.to_string()],
        ),
    ];

    for (name, input, expected) in cases {
        let pages = prepare_completed_pages(&input).map_err(|error| format!("{error:?}"))?;
        let expected = expected.iter().map(String::as_str).collect::<Vec<_>>();
        assert_eq!(prepared_texts(&pages), expected, "{name}");
        assert_eq!(concat_prepared(&pages), input, "{name} lost content");
    }

    Ok(())
}

#[test]
fn completed_layout_round_trips_other_unicode_best_effort() -> Result<(), String> {
    let arabic_input = format!("{ARABIC_9}م");
    let arabic_pages =
        prepare_completed_pages(&arabic_input).map_err(|error| format!("{error:?}"))?;
    assert_eq!(prepared_texts(&arabic_pages), vec![ARABIC_9, "م"]);

    let wide_punctuation_input = format!("{WIDE_PUNCTUATION_9}⸻");
    let wide_punctuation_pages =
        prepare_completed_pages(&wide_punctuation_input).map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        prepared_texts(&wide_punctuation_pages),
        vec![WIDE_PUNCTUATION_9, "⸻"]
    );

    let standalone_skin_tone_input = format!("{STANDALONE_SKIN_TONE_9} 🏽 ");
    let standalone_skin_tone_pages = prepare_completed_pages(&standalone_skin_tone_input)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        prepared_texts(&standalone_skin_tone_pages),
        vec![STANDALONE_SKIN_TONE_9, " 🏽 "]
    );

    let inputs = [
        "مرحبا بالعالم 👋🏽 ".repeat(20),
        "नमस्ते दुनिया 🇮🇳 ".repeat(20),
        "ภาษาไทยทดสอบ 🧑‍💻 ".repeat(20),
    ];

    for input in inputs {
        let pages = prepare_completed_pages(&input).map_err(|error| format!("{error:?}"))?;
        let boundaries = input
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .chain(std::iter::once(input.len()))
            .collect::<HashSet<_>>();
        let mut byte_offset = 0;

        assert!(!pages.is_empty());
        for page in &pages {
            assert!(!page.as_str().is_empty());
            assert!(page.as_str().encode_utf16().count() <= CHATBOX_MAX_UTF16_UNITS);
            let standalone_pages =
                prepare_completed_pages(page.as_str()).map_err(|error| format!("{error:?}"))?;
            assert_eq!(prepared_texts(&standalone_pages), vec![page.as_str()]);
            byte_offset += page.as_str().len();
            assert!(boundaries.contains(&byte_offset));
        }
        assert_eq!(concat_prepared(&pages), input);
    }

    Ok(())
}

#[test]
fn completed_layout_rejects_one_grapheme_larger_than_vrchat_input() {
    let oversized = format!("e{}", "\u{301}".repeat(CHATBOX_MAX_UTF16_UNITS));

    assert_eq!(oversized.graphemes(true).count(), 1);
    assert_eq!(
        prepare_completed_pages(&oversized),
        Err(ChatboxLayoutError::GraphemeExceedsInputBudget {
            utf16_units: CHATBOX_MAX_UTF16_UNITS + 1,
        })
    );
}

#[test]
fn live_viewport_keeps_a_full_recent_ascii_suffix_instead_of_the_last_completed_page()
-> Result<(), String> {
    let input = format!("{X_144}x");
    let viewport = require_live_view(&input)?;

    assert_eq!(viewport.as_str(), X_144);
    assert_ne!(
        viewport.as_str(),
        prepare_completed_pages(&input).map_err(|error| format!("{error:?}"))?[1].as_str()
    );

    Ok(())
}

#[test]
fn live_viewport_prefers_a_recent_word_and_punctuation_boundary() -> Result<(), String> {
    let input = format!("{X_144} previous context. latest, newest.");
    let viewport = require_live_view(&input)?;

    assert_eq!(viewport.as_str(), "previous context. latest, newest.");
    let pages = prepare_completed_pages(viewport.as_str()).map_err(|error| format!("{error:?}"))?;
    assert_eq!(prepared_texts(&pages), vec![viewport.as_str()]);

    Ok(())
}

#[test]
fn live_viewport_keeps_the_newest_nine_lines_without_a_leading_blank_line() -> Result<(), String> {
    let input = (1..=10)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let expected = (2..=10)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>()
        .join("\n");

    let viewport = require_live_view(&input)?;

    assert_eq!(viewport.as_str(), expected);
    let pages = prepare_completed_pages(viewport.as_str()).map_err(|error| format!("{error:?}"))?;
    assert_eq!(prepared_texts(&pages), vec![viewport.as_str()]);

    Ok(())
}

#[test]
fn live_viewport_keeps_the_newest_chinese_content_within_nine_lines() -> Result<(), String> {
    let input = format!("{CJK_135}新");
    let expected = format!("{CJK_134}新");
    let viewport = require_live_view(&input)?;

    assert_eq!(viewport.as_str(), expected);
    assert!(viewport.as_str().ends_with('新'));
    let pages = prepare_completed_pages(viewport.as_str()).map_err(|error| format!("{error:?}"))?;
    assert_eq!(prepared_texts(&pages), vec![viewport.as_str()]);

    Ok(())
}

#[test]
fn live_viewport_never_splits_an_emoji_grapheme() -> Result<(), String> {
    let input = format!("👍🏽{TONED_EMOJI_9}");
    let viewport = require_live_view(&input)?;

    assert_eq!(viewport.as_str(), TONED_EMOJI_9);
    assert_eq!(viewport.as_str().graphemes(true).count(), 9);
    let pages = prepare_completed_pages(viewport.as_str()).map_err(|error| format!("{error:?}"))?;
    assert_eq!(prepared_texts(&pages), vec![viewport.as_str()]);

    Ok(())
}

#[test]
fn live_viewport_falls_back_to_a_grapheme_boundary_for_one_long_token() -> Result<(), String> {
    let input = format!("x{X_144}");
    let viewport = require_live_view(&input)?;

    assert_eq!(viewport.as_str(), X_144);
    assert!(input.ends_with(viewport.as_str()));
    assert_eq!(
        viewport.as_str().encode_utf16().count(),
        CHATBOX_MAX_UTF16_UNITS
    );

    Ok(())
}

#[test]
fn live_viewport_preserves_tmp_punctuation_seams_at_its_start() -> Result<(), String> {
    let input = format!("{CJK_135}。");
    let viewport = require_live_view(&input)?;

    assert!(viewport.as_str().ends_with("中。"));
    assert!(!viewport.as_str().starts_with('。'));
    let pages = prepare_completed_pages(viewport.as_str()).map_err(|error| format!("{error:?}"))?;
    assert_eq!(prepared_texts(&pages), vec![viewport.as_str()]);

    Ok(())
}

#[test]
fn live_viewport_discards_an_unrepresentable_old_grapheme_and_keeps_new_content()
-> Result<(), String> {
    let oversized = format!("e{}", "\u{301}".repeat(CHATBOX_MAX_UTF16_UNITS));
    let input = format!("{oversized} newest");

    let viewport = require_live_view(&input)?;

    assert_eq!(viewport.as_str(), " newest");
    let pages = prepare_completed_pages(viewport.as_str()).map_err(|error| format!("{error:?}"))?;
    assert_eq!(prepared_texts(&pages), vec![viewport.as_str()]);
    Ok(())
}

#[test]
fn live_viewport_preserves_prepared_separators_after_oversized_history() -> Result<(), String> {
    let oversized = format!("e{}", "\u{301}".repeat(CHATBOX_MAX_UTF16_UNITS));
    let input = format!("{oversized}\r\n\rnew");

    let viewport = require_live_view(&input)?;

    assert_eq!(viewport.as_str(), "\r\n new");
    Ok(())
}

#[test]
fn live_viewport_rejects_an_unrepresentable_newest_grapheme() {
    let oversized = format!("e{}", "\u{301}".repeat(CHATBOX_MAX_UTF16_UNITS));

    assert_eq!(
        prepare_live_viewport(&format!("older {oversized}")),
        Err(ChatboxLayoutError::GraphemeExceedsInputBudget {
            utf16_units: CHATBOX_MAX_UTF16_UNITS + 1,
        })
    );
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: LAYOUT_PROPERTY_CASES,
        max_shrink_iters: 256,
        ..ProptestConfig::default()
    })]

    #[test]
    fn completed_pagination_is_lossless_bounded_nonempty_and_deterministic(
        input in representative_grapheme_text(),
    ) {
        let first = prepare_completed_pages(&input);
        let second = prepare_completed_pages(&input);

        prop_assert_eq!(&first, &second);
        match first {
            Ok(pages) => {
                prop_assert_eq!(pages.is_empty(), input.is_empty());
                prop_assert!(pages.iter().all(|page| !page.as_str().is_empty()));
                let every_page_is_within_budget = pages.iter().all(|page| {
                    page.as_str().encode_utf16().count() <= CHATBOX_MAX_UTF16_UNITS
                });
                prop_assert!(every_page_is_within_budget);
                prop_assert_eq!(concat_prepared(&pages), input);
            }
            Err(ChatboxLayoutError::GraphemeExceedsInputBudget { utf16_units }) => {
                prop_assert!(utf16_units > CHATBOX_MAX_UTF16_UNITS);
                let contains_oversized_grapheme = input.graphemes(true).any(|grapheme| {
                    grapheme.encode_utf16().count() > CHATBOX_MAX_UTF16_UNITS
                });
                prop_assert!(contains_oversized_grapheme);
            }
            Err(ChatboxLayoutError::RequiresPagination { page_count }) => {
                prop_assert!(false, "Completed pagination returned its single-view-only error for {page_count} pages");
            }
        }
    }

    #[test]
    fn control_preparation_is_bounded_and_matches_the_authored_oracle(
        (raw, expected) in prepared_control_policy_text(),
    ) {
        let result = prepare_completed_pages(&raw);
        prop_assert!(
            result.is_ok(),
            "authored control-policy atoms must be representable: {result:?}"
        );
        let pages = result.unwrap_or_default();
        let pages_are_nonempty_and_bounded = pages.iter().all(|page| {
            !page.as_str().is_empty()
                && page.as_str().encode_utf16().count() <= CHATBOX_MAX_UTF16_UNITS
        });
        let prepared_boundaries = expected
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .chain(std::iter::once(expected.len()))
            .collect::<HashSet<_>>();
        let mut prepared_offset = 0;
        let every_page_ends_at_a_prepared_grapheme = pages.iter().all(|page| {
            prepared_offset += page.as_str().len();
            prepared_boundaries.contains(&prepared_offset)
        });

        prop_assert_eq!(concat_prepared(&pages), expected);
        prop_assert!(pages_are_nonempty_and_bounded || raw.is_empty());
        prop_assert!(every_page_ends_at_a_prepared_grapheme);
    }

    #[test]
    fn live_viewport_is_bounded_and_deterministic(
        input in representative_grapheme_text(),
    ) {
        let first = prepare_live_viewport(&input);
        let second = prepare_live_viewport(&input);

        prop_assert_eq!(&first, &second);
        if let Ok(Some(viewport)) = first {
            prop_assert!(
                viewport.as_str().encode_utf16().count() <= CHATBOX_MAX_UTF16_UNITS
            );
        }
    }
}
