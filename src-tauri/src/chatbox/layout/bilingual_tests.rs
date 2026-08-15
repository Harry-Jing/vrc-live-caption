use super::{PreparedBilingualCompletedPage, prepare_bilingual_completed_pages};
use crate::chatbox::PreparedChatboxText;
use crate::chatbox::layout::{
    CHATBOX_MAX_UTF16_UNITS, ChatboxLayoutError, prepare_completed_pages, prepare_single_message,
};
use proptest::prelude::*;
use unicode_segmentation::UnicodeSegmentation;

const BILINGUAL_PROPERTY_CASES: u32 = 64;
const MAX_GENERATED_LANE_ATOMS: usize = 160;

fn prepared_lane_text() -> impl Strategy<Value = (String, String)> {
    let atom = prop::sample::select(vec![
        ("a", "a"),
        ("W", "W"),
        ("中", "中"),
        ("語", "語"),
        ("e\u{301}", "e\u{301}"),
        ("👨‍👩‍👧‍👦", "👨‍👩‍👧‍👦"),
        ("🧑‍💻", "🧑‍💻"),
        ("👍🏽", "👍🏽"),
        ("\r", " "),
        ("\u{0085}", " "),
        ("\u{000C}", " "),
        ("\r\n", "\r\n"),
        ("x\n", "x\n"),
        ("\u{000B}", "\u{000B}"),
        ("\u{2028}", "\u{2028}"),
        ("\u{2029}", "\u{2029}"),
        (" ", " "),
        (" \u{FE0F}", " \u{FE0F}"),
        ("「", "「"),
        ("。", "。"),
        ("’", "’"),
        ("—", "—"),
        ("(", "("),
        (")", ")"),
    ]);

    prop::collection::vec(atom, 0..=MAX_GENERATED_LANE_ATOMS).prop_map(|atoms| {
        let raw = atoms.iter().map(|(raw, _)| *raw).collect::<String>();
        let prepared = atoms
            .iter()
            .map(|(_, prepared)| *prepared)
            .collect::<String>();
        (raw, prepared)
    })
}

#[test]
fn short_pair_prepares_each_lane_before_sealing_the_payload() -> Result<(), String> {
    let mut pages = prepare_bilingual_completed_pages("Source\r", "翻译\u{0085}")
        .map_err(|error| format!("{error:?}"))?;

    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].prepared_source_text(), "Source ");
    assert_eq!(pages[0].prepared_translation_text(), "翻译 ");
    assert_eq!(pages[0].prepared_text().as_str(), "Source \n翻译 ");

    let consume: fn(PreparedBilingualCompletedPage) -> PreparedChatboxText =
        PreparedBilingualCompletedPage::into_prepared_text;
    let prepared = consume(pages.remove(0));
    assert_eq!(prepared.as_str(), "Source \n翻译 ");

    Ok(())
}

#[test]
fn shared_pages_use_the_private_four_five_baseline_and_donate_spare_lines() -> Result<(), String> {
    let long_source = "中".repeat(180);
    let long_translation = "文".repeat(180);
    let pages = prepare_bilingual_completed_pages(&long_source, &long_translation)
        .map_err(|error| format!("{error:?}"))?;

    assert_eq!(pages[0].prepared_source_text(), "中".repeat(60));
    assert_eq!(pages[0].prepared_translation_text(), "文".repeat(75));

    let pages = prepare_bilingual_completed_pages("源", &long_translation)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(pages[0].prepared_source_text(), "源");
    assert_eq!(pages[0].prepared_translation_text(), "文".repeat(120));

    let pages = prepare_bilingual_completed_pages(&long_source, "译")
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(pages[0].prepared_source_text(), "中".repeat(120));
    assert_eq!(pages[0].prepared_translation_text(), "译");

    Ok(())
}

#[test]
fn shared_utf16_capacity_uses_the_four_five_baseline_then_translation_first_donation()
-> Result<(), String> {
    let long_source = "x".repeat(180);
    let long_translation = "y".repeat(180);
    let pages = prepare_bilingual_completed_pages(&long_source, &long_translation)
        .map_err(|error| format!("{error:?}"))?;

    assert_eq!(pages[0].prepared_source_text(), "x".repeat(63));
    assert_eq!(pages[0].prepared_translation_text(), "y".repeat(80));

    let pages = prepare_bilingual_completed_pages("s", &long_translation)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(pages[0].prepared_source_text(), "s");
    assert_eq!(pages[0].prepared_translation_text(), "y".repeat(142));

    Ok(())
}

#[test]
fn two_individually_safe_graphemes_that_cannot_share_advance_source_first() -> Result<(), String> {
    let blocking_source = format!("s{}", "\u{301}".repeat(71));
    let blocking_translation = format!("t{}", "\u{301}".repeat(71));
    let source = format!("{blocking_source}S");
    let translation = format!("{blocking_translation}T");

    assert_eq!(blocking_source.graphemes(true).count(), 1);
    assert_eq!(blocking_translation.graphemes(true).count(), 1);
    assert_eq!(blocking_source.encode_utf16().count(), 72);
    assert_eq!(blocking_translation.encode_utf16().count(), 72);

    let pages = prepare_bilingual_completed_pages(&source, &translation)
        .map_err(|error| format!("{error:?}"))?;

    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].prepared_source_text(), blocking_source);
    assert!(pages[0].prepared_translation_text().is_empty());
    assert_eq!(pages[1].prepared_source_text(), "S");
    assert_eq!(pages[1].prepared_translation_text(), translation);

    Ok(())
}

#[test]
fn exhausted_lanes_never_repeat_and_the_long_lane_uses_monolingual_tail_pages() -> Result<(), String>
{
    let translation = "文".repeat(300);
    let pages = prepare_bilingual_completed_pages("source", &translation)
        .map_err(|error| format!("{error:?}"))?;
    let first_translation_tail = pages
        .iter()
        .position(|page| page.prepared_source_text().is_empty())
        .ok_or_else(|| "expected Translation-only tail pages".to_owned())?;

    assert!(
        pages[first_translation_tail..]
            .iter()
            .all(|page| page.prepared_source_text().is_empty())
    );
    let shared_translation = pages[..first_translation_tail]
        .iter()
        .map(PreparedBilingualCompletedPage::prepared_translation_text)
        .collect::<String>();
    let remaining_translation = translation
        .strip_prefix(&shared_translation)
        .ok_or_else(|| "shared pages were not a Translation prefix".to_owned())?;
    let expected_translation_tail =
        prepare_completed_pages(remaining_translation).map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        pages[first_translation_tail..]
            .iter()
            .map(|page| page.prepared_text().as_str())
            .collect::<Vec<_>>(),
        expected_translation_tail
            .iter()
            .map(PreparedChatboxText::as_str)
            .collect::<Vec<_>>()
    );

    let source = "中".repeat(300);
    let pages = prepare_bilingual_completed_pages(&source, "translation")
        .map_err(|error| format!("{error:?}"))?;
    let first_source_tail = pages
        .iter()
        .position(|page| page.prepared_translation_text().is_empty())
        .ok_or_else(|| "expected Source-only tail pages".to_owned())?;

    assert!(
        pages[first_source_tail..]
            .iter()
            .all(|page| page.prepared_translation_text().is_empty())
    );
    let shared_source = pages[..first_source_tail]
        .iter()
        .map(PreparedBilingualCompletedPage::prepared_source_text)
        .collect::<String>();
    let remaining_source = source
        .strip_prefix(&shared_source)
        .ok_or_else(|| "shared pages were not a Source prefix".to_owned())?;
    let expected_source_tail =
        prepare_completed_pages(remaining_source).map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        pages[first_source_tail..]
            .iter()
            .map(|page| page.prepared_text().as_str())
            .collect::<Vec<_>>(),
        expected_source_tail
            .iter()
            .map(PreparedChatboxText::as_str)
            .collect::<Vec<_>>()
    );

    Ok(())
}

#[test]
fn empty_lane_cases_are_exactly_the_existing_monolingual_completed_layout() -> Result<(), String> {
    assert!(
        prepare_bilingual_completed_pages("", "")
            .map_err(|error| format!("{error:?}"))?
            .is_empty()
    );

    let source = format!("{}\r{}", "x".repeat(143), "中".repeat(136));
    let source_only = prepare_completed_pages(&source).map_err(|error| format!("{error:?}"))?;
    let bilingual =
        prepare_bilingual_completed_pages(&source, "").map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        bilingual
            .iter()
            .map(|page| page.prepared_text().as_str())
            .collect::<Vec<_>>(),
        source_only
            .iter()
            .map(PreparedChatboxText::as_str)
            .collect::<Vec<_>>()
    );
    assert!(
        bilingual
            .iter()
            .all(|page| page.prepared_translation_text().is_empty())
    );

    let translation = format!("{}\u{0085}{}", "文".repeat(136), "y".repeat(145));
    let translation_only =
        prepare_completed_pages(&translation).map_err(|error| format!("{error:?}"))?;
    let bilingual = prepare_bilingual_completed_pages("", &translation)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        bilingual
            .iter()
            .map(|page| page.prepared_text().as_str())
            .collect::<Vec<_>>(),
        translation_only
            .iter()
            .map(PreparedChatboxText::as_str)
            .collect::<Vec<_>>()
    );
    assert!(
        bilingual
            .iter()
            .all(|page| page.prepared_source_text().is_empty())
    );

    Ok(())
}

#[test]
fn exact_combined_payload_is_revalidated_after_lane_local_planning() -> Result<(), String> {
    let source = "s\ns\ns\ns\n";
    let translation = "t\nt\nt\nt\nt";
    let pages = prepare_bilingual_completed_pages(source, translation)
        .map_err(|error| format!("{error:?}"))?;

    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].prepared_source_text(), "s\ns\ns\ns");
    assert_eq!(pages[0].prepared_translation_text(), translation);
    assert_eq!(pages[1].prepared_source_text(), "\n");
    assert!(pages[1].prepared_translation_text().is_empty());
    for page in pages {
        assert_eq!(
            prepare_single_message(page.prepared_text().as_str()),
            Ok(Some(page.prepared_text().clone()))
        );
    }

    Ok(())
}

#[test]
fn bilingual_layout_matches_language_grapheme_and_boundary_fixtures() -> Result<(), String> {
    struct Case {
        name: &'static str,
        source: String,
        translation: String,
    }

    let cases = [
        Case {
            name: "short pair",
            source: "Hello!".to_owned(),
            translation: "你好！".to_owned(),
        },
        Case {
            name: "long Source token",
            source: "x".repeat(145),
            translation: "较长的原文会在后续单语页继续。".to_owned(),
        },
        Case {
            name: "long Translation token",
            source: "Short source.".to_owned(),
            translation: "y".repeat(145),
        },
        Case {
            name: "mixed English and Chinese",
            source: "VRChat 中的 mixed caption 42.".to_owned(),
            translation: "VRChat mixed 字幕，第 42 条。".to_owned(),
        },
        Case {
            name: "punctuation",
            source: "Source (exact): ‘hello’ — done.".to_owned(),
            translation: "翻译（精确）：‘你好’——完成。".to_owned(),
        },
        Case {
            name: "emoji graphemes",
            source: "Family 👨‍👩‍👧‍👦 and coder 🧑‍💻".to_owned(),
            translation: "一家人 👨‍👩‍👧‍👦 和开发者 🧑‍💻 👍🏽".to_owned(),
        },
        Case {
            name: "boundary-sized inputs",
            source: "x".repeat(CHATBOX_MAX_UTF16_UNITS),
            translation: "中".repeat(135),
        },
    ];

    for case in cases {
        let first = prepare_bilingual_completed_pages(&case.source, &case.translation)
            .map_err(|error| format!("{}: {error:?}", case.name))?;
        let second = prepare_bilingual_completed_pages(&case.source, &case.translation)
            .map_err(|error| format!("{}: {error:?}", case.name))?;

        assert_eq!(first, second, "{} was not deterministic", case.name);
        assert_eq!(
            first
                .iter()
                .map(PreparedBilingualCompletedPage::prepared_source_text)
                .collect::<String>(),
            case.source,
            "{} lost Source text",
            case.name
        );
        assert_eq!(
            first
                .iter()
                .map(PreparedBilingualCompletedPage::prepared_translation_text)
                .collect::<String>(),
            case.translation,
            "{} lost Translation text",
            case.name
        );
        assert!(
            first.iter().all(|page| {
                prepare_single_message(page.prepared_text().as_str())
                    == Ok(Some(page.prepared_text().clone()))
            }),
            "{} emitted a page that was unsafe on its own",
            case.name
        );
    }

    Ok(())
}

#[test]
fn oversized_graphemes_fail_the_whole_pair_before_any_pages_escape() {
    let oversized = format!("e{}", "\u{301}".repeat(CHATBOX_MAX_UTF16_UNITS));
    let expected = Err(ChatboxLayoutError::GraphemeExceedsInputBudget {
        utf16_units: CHATBOX_MAX_UTF16_UNITS + 1,
    });

    assert_eq!(
        prepare_bilingual_completed_pages(&oversized, "translation"),
        expected
    );
    assert_eq!(
        prepare_bilingual_completed_pages("source", &oversized),
        expected
    );
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: BILINGUAL_PROPERTY_CASES,
        max_shrink_iters: 256,
        ..ProptestConfig::default()
    })]

    #[test]
    fn prepared_lanes_reconstruct_once_and_every_page_is_independently_safe(
        (raw_source, prepared_source) in prepared_lane_text(),
        (raw_translation, prepared_translation) in prepared_lane_text(),
    ) {
        let first = prepare_bilingual_completed_pages(&raw_source, &raw_translation);
        let second = prepare_bilingual_completed_pages(&raw_source, &raw_translation);

        prop_assert_eq!(&first, &second);
        match first {
            Ok(pages) => {
                let reconstructed_source = pages
                    .iter()
                    .map(PreparedBilingualCompletedPage::prepared_source_text)
                    .collect::<String>();
                let reconstructed_translation = pages
                    .iter()
                    .map(PreparedBilingualCompletedPage::prepared_translation_text)
                    .collect::<String>();
                prop_assert_eq!(&reconstructed_source, &prepared_source);
                prop_assert_eq!(&reconstructed_translation, &prepared_translation);

                let mut source_consumed = 0;
                let mut translation_consumed = 0;
                for page in &pages {
                    let source = page.prepared_source_text();
                    let translation = page.prepared_translation_text();
                    let payload = page.prepared_text();

                    prop_assert!(!payload.as_str().is_empty());
                    prop_assert!(
                        payload.as_str().encode_utf16().count()
                            <= CHATBOX_MAX_UTF16_UNITS
                    );
                    prop_assert_eq!(
                        prepare_single_message(payload.as_str()),
                        Ok(Some(payload.clone()))
                    );
                    match (source.is_empty(), translation.is_empty()) {
                        (false, false) => prop_assert_eq!(
                            payload.as_str(),
                            format!("{source}\n{translation}")
                        ),
                        (false, true) => prop_assert_eq!(payload.as_str(), source),
                        (true, false) => prop_assert_eq!(payload.as_str(), translation),
                        (true, true) => prop_assert!(false, "layout emitted an empty page"),
                    }

                    if source_consumed == prepared_source.len() {
                        prop_assert!(source.is_empty());
                    }
                    if translation_consumed == prepared_translation.len() {
                        prop_assert!(translation.is_empty());
                    }
                    source_consumed += source.len();
                    translation_consumed += translation.len();
                }
            }
            Err(ChatboxLayoutError::GraphemeExceedsInputBudget { utf16_units }) => {
                prop_assert!(utf16_units > CHATBOX_MAX_UTF16_UNITS);
            }
            Err(ChatboxLayoutError::RequiresPagination { page_count }) => {
                prop_assert!(false, "bilingual pagination leaked a {page_count}-page single-message error");
            }
        }
    }
}
