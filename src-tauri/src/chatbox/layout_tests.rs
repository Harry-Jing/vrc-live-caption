use super::{
    CHATBOX_MAX_UTF16_UNITS, ChatboxLayoutError, fits_chatbox_width, grapheme_advance_units,
    is_break_space_grapheme, paginate_completed, render_live_viewport,
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
const HELLO_VS_20_WITH_SPACE: &str = concat!(
    "hello \u{FE0F}hello \u{FE0F}hello \u{FE0F}hello \u{FE0F}hello \u{FE0F}",
    "hello \u{FE0F}hello \u{FE0F}hello \u{FE0F}hello \u{FE0F}hello \u{FE0F}",
    "hello \u{FE0F}hello \u{FE0F}hello \u{FE0F}hello \u{FE0F}hello \u{FE0F}",
    "hello \u{FE0F}hello \u{FE0F}hello \u{FE0F}hello \u{FE0F}hello \u{FE0F}",
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
        "\u{2028}",
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

#[test]
fn completed_layout_preserves_explicit_line_breaks() -> Result<(), String> {
    let cases = [
        ("LF", "one\ntwo\n中", "one\ntwo\n中"),
        ("CRLF", "one\r\ntwo", "one\r\ntwo"),
        ("CR", "one\rtwo", "one\rtwo"),
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
        let pages = paginate_completed(input).map_err(|error| format!("{error:?}"))?;
        assert_eq!(pages, vec![expected.to_string()], "{name}");
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
    let pages = paginate_completed(&input).map_err(|error| format!("{error:?}"))?;

    assert_eq!(pages, vec![CJK_135.to_string(), "中".to_string()]);

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
            name: "space modifier preserves the English word boundary",
            input: format!("{HELLO_VS_20_WITH_SPACE}{X_30}"),
            expected: vec![HELLO_VS_20_WITH_SPACE.to_string(), X_30.to_string()],
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
        let pages = paginate_completed(&case.input).map_err(|error| format!("{error:?}"))?;
        assert_eq!(pages, case.expected, "{}", case.name);
        assert_eq!(pages.concat(), case.input, "{} lost content", case.name);
        assert!(
            pages
                .iter()
                .all(|page| page.encode_utf16().count() <= CHATBOX_MAX_UTF16_UNITS),
            "{} exceeded the UTF-16 budget",
            case.name
        );
        for page in &pages {
            let standalone_pages =
                paginate_completed(page).map_err(|error| format!("{error:?}"))?;
            assert_eq!(
                standalone_pages,
                vec![page.clone()],
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
        let pages = paginate_completed(&input).map_err(|error| format!("{error:?}"))?;
        assert_eq!(pages, expected, "{name}");
        assert_eq!(pages.concat(), input, "{name} lost content");
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
        let pages = paginate_completed(&input).map_err(|error| format!("{error:?}"))?;
        assert_eq!(pages, expected, "{name}");
        assert_eq!(pages.concat(), input, "{name} lost content");
    }

    Ok(())
}

#[test]
fn completed_layout_round_trips_other_unicode_best_effort() -> Result<(), String> {
    let arabic_input = format!("{ARABIC_9}م");
    let arabic_pages = paginate_completed(&arabic_input).map_err(|error| format!("{error:?}"))?;
    assert_eq!(arabic_pages, vec![ARABIC_9.to_string(), "م".to_string()]);

    let wide_punctuation_input = format!("{WIDE_PUNCTUATION_9}⸻");
    let wide_punctuation_pages =
        paginate_completed(&wide_punctuation_input).map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        wide_punctuation_pages,
        vec![WIDE_PUNCTUATION_9.to_string(), "⸻".to_string()]
    );

    let standalone_skin_tone_input = format!("{STANDALONE_SKIN_TONE_9} 🏽 ");
    let standalone_skin_tone_pages =
        paginate_completed(&standalone_skin_tone_input).map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        standalone_skin_tone_pages,
        vec![STANDALONE_SKIN_TONE_9.to_string(), " 🏽 ".to_string()]
    );

    let inputs = [
        "مرحبا بالعالم 👋🏽 ".repeat(20),
        "नमस्ते दुनिया 🇮🇳 ".repeat(20),
        "ภาษาไทยทดสอบ 🧑‍💻 ".repeat(20),
    ];

    for input in inputs {
        let pages = paginate_completed(&input).map_err(|error| format!("{error:?}"))?;
        let boundaries = input
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .chain(std::iter::once(input.len()))
            .collect::<HashSet<_>>();
        let mut byte_offset = 0;

        assert!(!pages.is_empty());
        for page in &pages {
            assert!(!page.is_empty());
            assert!(page.encode_utf16().count() <= CHATBOX_MAX_UTF16_UNITS);
            let standalone_pages =
                paginate_completed(page).map_err(|error| format!("{error:?}"))?;
            assert_eq!(standalone_pages, vec![page.clone()]);
            byte_offset += page.len();
            assert!(boundaries.contains(&byte_offset));
        }
        assert_eq!(pages.concat(), input);
    }

    Ok(())
}

#[test]
fn completed_layout_rejects_one_grapheme_larger_than_vrchat_input() {
    let oversized = format!("e{}", "\u{301}".repeat(CHATBOX_MAX_UTF16_UNITS));

    assert_eq!(oversized.graphemes(true).count(), 1);
    assert_eq!(
        paginate_completed(&oversized),
        Err(ChatboxLayoutError::GraphemeExceedsInputBudget {
            utf16_units: CHATBOX_MAX_UTF16_UNITS + 1,
        })
    );
}

#[test]
fn live_viewport_keeps_a_full_recent_ascii_suffix_instead_of_the_last_completed_page()
-> Result<(), String> {
    let input = format!("{X_144}x");
    let viewport = render_live_viewport(&input).map_err(|error| format!("{error:?}"))?;

    assert_eq!(viewport, X_144);
    assert_ne!(
        viewport,
        paginate_completed(&input).map_err(|error| format!("{error:?}"))?[1]
    );

    Ok(())
}

#[test]
fn live_viewport_prefers_a_recent_word_and_punctuation_boundary() -> Result<(), String> {
    let input = format!("{X_144} previous context. latest, newest.");
    let viewport = render_live_viewport(&input).map_err(|error| format!("{error:?}"))?;

    assert_eq!(viewport, "previous context. latest, newest.");
    assert_eq!(paginate_completed(&viewport), Ok(vec![viewport.clone()]));

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

    let viewport = render_live_viewport(&input).map_err(|error| format!("{error:?}"))?;

    assert_eq!(viewport, expected);
    assert_eq!(paginate_completed(&viewport), Ok(vec![viewport.clone()]));

    Ok(())
}

#[test]
fn live_viewport_keeps_the_newest_chinese_content_within_nine_lines() -> Result<(), String> {
    let input = format!("{CJK_135}新");
    let expected = format!("{CJK_134}新");
    let viewport = render_live_viewport(&input).map_err(|error| format!("{error:?}"))?;

    assert_eq!(viewport, expected);
    assert!(viewport.ends_with('新'));
    assert_eq!(paginate_completed(&viewport), Ok(vec![viewport.clone()]));

    Ok(())
}

#[test]
fn live_viewport_never_splits_an_emoji_grapheme() -> Result<(), String> {
    let input = format!("👍🏽{TONED_EMOJI_9}");
    let viewport = render_live_viewport(&input).map_err(|error| format!("{error:?}"))?;

    assert_eq!(viewport, TONED_EMOJI_9);
    assert_eq!(viewport.graphemes(true).count(), 9);
    assert_eq!(paginate_completed(&viewport), Ok(vec![viewport.clone()]));

    Ok(())
}

#[test]
fn live_viewport_falls_back_to_a_grapheme_boundary_for_one_long_token() -> Result<(), String> {
    let input = format!("x{X_144}");
    let viewport = render_live_viewport(&input).map_err(|error| format!("{error:?}"))?;

    assert_eq!(viewport, X_144);
    assert!(input.ends_with(&viewport));
    assert_eq!(viewport.encode_utf16().count(), CHATBOX_MAX_UTF16_UNITS);

    Ok(())
}

#[test]
fn live_viewport_preserves_tmp_punctuation_seams_at_its_start() -> Result<(), String> {
    let input = format!("{CJK_135}。");
    let viewport = render_live_viewport(&input).map_err(|error| format!("{error:?}"))?;

    assert!(viewport.ends_with("中。"));
    assert!(!viewport.starts_with('。'));
    assert_eq!(paginate_completed(&viewport), Ok(vec![viewport.clone()]));

    Ok(())
}

#[test]
fn live_viewport_discards_an_unrepresentable_old_grapheme_and_keeps_new_content()
-> Result<(), String> {
    let oversized = format!("e{}", "\u{301}".repeat(CHATBOX_MAX_UTF16_UNITS));
    let input = format!("{oversized} newest");

    let viewport = render_live_viewport(&input).map_err(|error| format!("{error:?}"))?;

    assert_eq!(viewport, "newest");
    assert_eq!(paginate_completed(&viewport), Ok(vec![viewport.clone()]));
    Ok(())
}

#[test]
fn live_viewport_rejects_an_unrepresentable_newest_grapheme() {
    let oversized = format!("e{}", "\u{301}".repeat(CHATBOX_MAX_UTF16_UNITS));

    assert_eq!(
        render_live_viewport(&format!("older {oversized}")),
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
        let first = paginate_completed(&input);
        let second = paginate_completed(&input);

        prop_assert_eq!(&first, &second);
        match first {
            Ok(pages) => {
                prop_assert_eq!(pages.is_empty(), input.is_empty());
                prop_assert!(pages.iter().all(|page| !page.is_empty()));
                let every_page_is_within_budget = pages.iter().all(|page| {
                    page.encode_utf16().count() <= CHATBOX_MAX_UTF16_UNITS
                });
                prop_assert!(every_page_is_within_budget);
                prop_assert_eq!(pages.concat(), input);
            }
            Err(ChatboxLayoutError::GraphemeExceedsInputBudget { utf16_units }) => {
                prop_assert!(utf16_units > CHATBOX_MAX_UTF16_UNITS);
                let contains_oversized_grapheme = input.graphemes(true).any(|grapheme| {
                    grapheme.encode_utf16().count() > CHATBOX_MAX_UTF16_UNITS
                });
                prop_assert!(contains_oversized_grapheme);
            }
        }
    }

    #[test]
    fn live_viewport_is_bounded_and_deterministic(
        input in representative_grapheme_text(),
    ) {
        let first = render_live_viewport(&input);
        let second = render_live_viewport(&input);

        prop_assert_eq!(&first, &second);
        if let Ok(viewport) = first {
            prop_assert!(viewport.encode_utf16().count() <= CHATBOX_MAX_UTF16_UNITS);
        }
    }
}
