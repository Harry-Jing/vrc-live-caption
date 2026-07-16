//! Pure text layout for VRChat Chatbox Completed pages.
//!
//! The module has no runtime, pacing, OSC, or queue dependencies. It simulates
//! VRChat's fixed 280 px TextMeshPro width, nine visible lines, and conservative
//! 144 UTF-16 input budget, then returns every page in source order. Soft wraps
//! choose page boundaries but are not inserted into the returned text; explicit
//! source line breaks and all other graphemes remain unchanged. Unsupported
//! Unicode graphemes conservatively reserve a whole line. Every candidate page
//! is revalidated from start-of-text context before it is returned.

use std::collections::HashMap;
use unicode_linebreak::{BreakOpportunity, linebreaks};
use unicode_segmentation::UnicodeSegmentation;

const CHATBOX_MAX_UTF16_UNITS: usize = 144;
const CHATBOX_MAX_VISIBLE_LINES: usize = 9;
const FONT_UNITS_PER_EM: u32 = 1_000;
const FONT_SIZE_PX: u32 = 18;
const CHATBOX_WIDTH_PX: u32 = 280;
const MAX_GRAPHEME_ADVANCE_UNITS: u32 = CHATBOX_WIDTH_PX * FONT_UNITS_PER_EM / FONT_SIZE_PX;
// TextMeshPro's extracted `Leading Characters` table: prefer not to leave one
// of these characters at the end of a wrapped line.
const TMP_LEADING_CHARACTERS: &str =
    r##"([｛〔〈《「『【〘〖〝‘“｟«$—…‥〳〴〵\［（{£¥"々〇〉》」＄｠￥￦ #"##;
// TextMeshPro's extracted `Following Characters` table: prefer not to start a
// wrapped line with one of these characters.
const TMP_FOLLOWING_CHARACTERS: &str = r##")]｝〕〉》」』】〙〗〟’”｠»ヽヾーァィゥェォッャュョヮヵヶぁぃぅぇぉっゃゅょゎゕゖㇰㇱㇲㇳㇴㇵㇶㇷㇸㇹㇺㇻㇼㇽㇾㇿ々〻‐゠–〜?!‼⁇⁈⁉・、%,.:;。！？］）：；＝}¢°"†‡℃〆％，．"##;

// NotoSans-Regular Version 2.000 horizontal advances for U+0020..U+007E.
// The font uses 1000 design units per em. These fixed metrics match the
// primary VRChat TMP font documented in the Chatbox reference and keep layout
// deterministic without depending on fonts installed on the user's machine.
#[rustfmt::skip]
const BASIC_LATIN_ADVANCES: [u16; 95] = [
    260, 269, 408, 646, 572, 831, 732, 225, 300, 300, 551, 572, 268, 322, 268, 372,
    572, 572, 572, 572, 572, 572, 572, 572, 572, 572, 268, 268, 572, 572, 572, 434,
    899, 639, 650, 632, 730, 556, 519, 728, 741, 339, 273, 619, 524, 907, 760, 781,
    605, 781, 622, 549, 556, 731, 600, 930, 586, 566, 572, 329, 372, 329, 572, 444,
    281, 561, 615, 480, 615, 564, 344, 615, 618, 258, 258, 534, 258, 935, 618, 605,
    615, 615, 413, 479, 361, 618, 508, 786, 529, 510, 470, 380, 551, 380, 572,
];

// NotoSans-Regular Version 2.000 horizontal advances for U+00A0..U+00FF.
#[rustfmt::skip]
const LATIN_1_ADVANCES: [u16; 96] = [
    260, 269, 572, 572, 572, 572, 551, 513, 580, 832, 357, 509, 572, 322, 832, 500,
    428, 572, 350, 350, 281, 623, 655, 268, 225, 350, 376, 509, 745, 771, 781, 434,
    639, 639, 639, 639, 639, 639, 881, 632, 556, 556, 556, 556, 339, 339, 339, 339,
    730, 760, 781, 781, 781, 781, 781, 572, 781, 731, 731, 731, 731, 566, 605, 631,
    561, 561, 561, 561, 561, 561, 864, 480, 564, 564, 564, 564, 258, 258, 258, 258,
    605, 618, 605, 605, 605, 605, 605, 572, 605, 618, 618, 618, 618, 510, 615, 510,
];

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ChatboxLayoutError {
    GraphemeExceedsInputBudget { utf16_units: usize },
}

/// Returns every safe Completed page in source order.
///
/// An empty caption has no pages. A single grapheme that is itself larger than
/// VRChat's complete input budget cannot be represented without violating one
/// of the layout invariants, so that pathological input returns an error.
pub(crate) fn paginate_completed(text: &str) -> Result<Vec<String>, ChatboxLayoutError> {
    if text.is_empty() {
        return Ok(Vec::new());
    }

    let prepared = PreparedText::new(text)?;
    let mut pages = Vec::new();
    let mut page_start = 0;

    while page_start < prepared.graphemes.len() {
        let proposed_end = prepared.next_page_end(page_start);
        let (page_end, page) = prepared.standalone_safe_page(page_start, proposed_end)?;
        pages.push(page);
        page_start = page_end;
    }

    Ok(pages)
}

struct PreparedText<'text> {
    graphemes: Vec<LayoutGrapheme<'text>>,
    utf16_prefix: Vec<usize>,
}

struct LayoutGrapheme<'text> {
    text: &'text str,
    advance_units: u32,
    explicit_line_break: bool,
    break_space: bool,
    can_break_after: bool,
}

impl<'text> PreparedText<'text> {
    fn new(text: &'text str) -> Result<Self, ChatboxLayoutError> {
        let break_opportunities = linebreaks(text).collect::<HashMap<_, _>>();
        let mut graphemes = Vec::new();
        let mut raw_breaks = Vec::new();
        let mut utf16_prefix = vec![0];

        for (start, grapheme) in text.grapheme_indices(true) {
            let utf16_units = grapheme.encode_utf16().count();
            if utf16_units > CHATBOX_MAX_UTF16_UNITS {
                return Err(ChatboxLayoutError::GraphemeExceedsInputBudget { utf16_units });
            }

            let explicit_line_break = is_explicit_line_break(grapheme);
            let break_space = !explicit_line_break && is_break_space_grapheme(grapheme);
            raw_breaks.push(grapheme_break_opportunity(
                &break_opportunities,
                start,
                grapheme,
                break_space,
            ));
            graphemes.push(LayoutGrapheme {
                text: grapheme,
                advance_units: if explicit_line_break {
                    0
                } else {
                    grapheme_advance_units(grapheme)
                },
                explicit_line_break,
                break_space,
                can_break_after: false,
            });
            let accumulated = utf16_prefix.last().copied().unwrap_or_default() + utf16_units;
            utf16_prefix.push(accumulated);
        }

        let (blocks_line_end, blocks_line_start) =
            tmp_blocking_characters_at_boundaries(&graphemes);
        for index in 0..graphemes.len() {
            graphemes[index].can_break_after = match raw_breaks[index] {
                Some(BreakOpportunity::Mandatory) => true,
                Some(BreakOpportunity::Allowed) => {
                    !blocks_line_end[index + 1] && !blocks_line_start[index + 1]
                }
                None => false,
            };
        }

        Ok(Self {
            graphemes,
            utf16_prefix,
        })
    }

    fn next_page_end(&self, page_start: usize) -> usize {
        let mut cursor = page_start;
        let mut line_start = page_start;
        let mut line_width = 0;
        let mut line_count = 1;
        let mut last_legal_break = None;
        let mut last_legal_page_break = None;

        while cursor < self.graphemes.len() {
            let grapheme = &self.graphemes[cursor];
            if self.utf16_units(page_start, cursor + 1) > CHATBOX_MAX_UTF16_UNITS {
                return last_legal_page_break
                    .filter(|boundary| *boundary > page_start)
                    .unwrap_or(cursor);
            }

            if grapheme.explicit_line_break {
                if line_count == CHATBOX_MAX_VISIBLE_LINES {
                    return cursor;
                }

                cursor += 1;
                last_legal_page_break = Some(cursor);
                line_count += 1;
                line_start = cursor;
                line_width = 0;
                last_legal_break = None;
                continue;
            }

            let candidate_width = line_width + grapheme.advance_units;
            if fits_chatbox_width(candidate_width) {
                line_width = candidate_width;
                cursor += 1;
                if grapheme.can_break_after {
                    last_legal_break = Some(cursor);
                    last_legal_page_break = Some(cursor);
                }
                continue;
            }

            let (line_end, line_end_is_legal) = if grapheme.break_space && grapheme.can_break_after
            {
                (cursor + 1, true)
            } else if let Some(boundary) =
                last_legal_break.filter(|boundary| *boundary > line_start)
            {
                (boundary, true)
            } else if cursor > line_start {
                (cursor, false)
            } else {
                (cursor + 1, false)
            };

            if line_count == CHATBOX_MAX_VISIBLE_LINES {
                return if line_end_is_legal {
                    line_end
                } else {
                    last_legal_page_break
                        .filter(|boundary| *boundary > page_start)
                        .unwrap_or(line_end)
                };
            }

            if line_end_is_legal {
                last_legal_page_break = Some(line_end);
            }
            line_count += 1;
            line_start = line_end;
            cursor = line_end;
            line_width = 0;
            last_legal_break = None;
        }

        self.graphemes.len()
    }

    fn standalone_safe_page(
        &self,
        page_start: usize,
        proposed_end: usize,
    ) -> Result<(usize, String), ChatboxLayoutError> {
        let mut page_end = proposed_end;

        loop {
            let candidate: String = self.graphemes[page_start..page_end]
                .iter()
                .map(|grapheme| grapheme.text)
                .collect();
            let safe_byte_len = {
                let standalone = PreparedText::new(&candidate)?;
                let standalone_end = standalone.next_page_end(0);
                (standalone_end != standalone.graphemes.len()).then(|| {
                    standalone.graphemes[..standalone_end]
                        .iter()
                        .map(|grapheme| grapheme.text.len())
                        .sum()
                })
            };
            let Some(safe_byte_len) = safe_byte_len else {
                return Ok((page_end, candidate));
            };
            let safer_end =
                self.page_end_at_or_before_byte_len(page_start, page_end, safe_byte_len);

            // A single source grapheme always fits: it was already checked
            // against the UTF-16 cap and its width is capped to one line.
            // This fallback also guarantees progress if standalone grapheme
            // boundaries ever differ from the source-context boundaries.
            page_end = if safer_end > page_start && safer_end < page_end {
                safer_end
            } else {
                page_start + 1
            };
        }
    }

    fn page_end_at_or_before_byte_len(
        &self,
        page_start: usize,
        page_end: usize,
        byte_len: usize,
    ) -> usize {
        let mut consumed = 0;
        let mut safe_end = page_start;

        for (offset, grapheme) in self.graphemes[page_start..page_end].iter().enumerate() {
            let next_consumed = consumed + grapheme.text.len();
            if next_consumed > byte_len {
                break;
            }
            consumed = next_consumed;
            safe_end = page_start + offset + 1;
        }

        safe_end
    }

    fn utf16_units(&self, start: usize, end: usize) -> usize {
        self.utf16_prefix[end] - self.utf16_prefix[start]
    }
}

fn grapheme_break_opportunity(
    break_opportunities: &HashMap<usize, BreakOpportunity>,
    start: usize,
    grapheme: &str,
    break_space: bool,
) -> Option<BreakOpportunity> {
    let end = start + grapheme.len();
    if let Some(opportunity) = break_opportunities.get(&end) {
        return Some(*opportunity);
    }

    if !break_space {
        return None;
    }

    // UAX #14 can place the opportunity immediately after a space while a
    // following variation selector keeps both scalars in one grapheme. Project
    // only that break-space opportunity to the safe grapheme boundary.
    grapheme
        .char_indices()
        .map(|(offset, character)| start + offset + character.len_utf8())
        .filter(|boundary| *boundary < end)
        .any(|boundary| {
            matches!(
                break_opportunities.get(&boundary),
                Some(BreakOpportunity::Allowed)
            )
        })
        .then_some(BreakOpportunity::Allowed)
}

fn tmp_blocking_characters_at_boundaries(
    graphemes: &[LayoutGrapheme<'_>],
) -> (Vec<bool>, Vec<bool>) {
    // Break-spaces are layout separators, not the visible edge characters TMP
    // is trying to keep away from line ends/starts. Check through a run of
    // spaces so `# ` cannot end a line and ` ’` cannot start the next one.
    let mut blocks_line_end = vec![false; graphemes.len() + 1];
    let mut nearest = None;
    for (index, grapheme) in graphemes.iter().enumerate() {
        if grapheme.explicit_line_break {
            nearest = None;
        } else if !grapheme.break_space
            && let Some(blocked) = tmp_edge_blocks(grapheme.text, TMP_LEADING_CHARACTERS)
        {
            nearest = Some(blocked);
        }
        blocks_line_end[index + 1] = nearest.unwrap_or(false);
    }

    let mut blocks_line_start = vec![false; graphemes.len() + 1];
    nearest = None;
    for (index, grapheme) in graphemes.iter().enumerate().rev() {
        if grapheme.explicit_line_break {
            nearest = None;
        } else if !grapheme.break_space
            && let Some(blocked) = tmp_edge_blocks(grapheme.text, TMP_FOLLOWING_CHARACTERS)
        {
            nearest = Some(blocked);
        }
        blocks_line_start[index] = nearest.unwrap_or(false);
    }

    (blocks_line_end, blocks_line_start)
}

fn tmp_edge_blocks(grapheme: &str, prohibited: &str) -> Option<bool> {
    let has_visible_content = grapheme
        .chars()
        .any(|character| !character.is_whitespace() && !is_zero_advance_modifier(character));
    has_visible_content.then(|| {
        grapheme
            .chars()
            .any(|character| prohibited.contains(character))
    })
}

fn is_break_space_grapheme(grapheme: &str) -> bool {
    let mut contains_space = false;
    let contains_only_space_and_zero_advance_modifiers = grapheme.chars().all(|character| {
        if character.is_whitespace() {
            contains_space = true;
            true
        } else {
            is_zero_advance_modifier(character)
        }
    });

    contains_space && contains_only_space_and_zero_advance_modifiers
}

fn fits_chatbox_width(advance_units: u32) -> bool {
    advance_units * FONT_SIZE_PX <= CHATBOX_WIDTH_PX * FONT_UNITS_PER_EM
}

fn grapheme_advance_units(grapheme: &str) -> u32 {
    grapheme
        .chars()
        .map(character_advance_units)
        .fold(0, u32::saturating_add)
        .min(MAX_GRAPHEME_ADVANCE_UNITS)
}

fn character_advance_units(character: char) -> u32 {
    if (' '..='~').contains(&character) {
        return u32::from(BASIC_LATIN_ADVANCES[character as usize - ' ' as usize]);
    }

    if ('\u{00A0}'..='\u{00FF}').contains(&character) {
        return u32::from(LATIN_1_ADVANCES[character as usize - 0x00A0]);
    }

    if let Some(advance) = common_noto_punctuation_advance(character) {
        return advance;
    }

    if character == '\t' {
        return u32::from(BASIC_LATIN_ADVANCES[0]) * 4;
    }

    if character.is_control() || is_zero_advance_modifier(character) {
        return 0;
    }

    if has_verified_chinese_fullwidth_advance(character) {
        // NotoSansCJK-JP-Regular uses a 1000-unit advance for the covered
        // Chinese ideographs and full-width punctuation.
        return FONT_UNITS_PER_EM;
    }

    // Unsupported graphemes reserve a whole line. A generic 1000-unit fallback
    // is unsafe because some Noto Sans glyphs are substantially wider than one
    // em; the full-line reservation keeps best-effort pagination conservative.
    MAX_GRAPHEME_ADVANCE_UNITS
}

fn common_noto_punctuation_advance(character: char) -> Option<u32> {
    Some(match character {
        '\u{2010}' | '\u{2011}' => 322,
        '\u{2012}' => 572,
        '\u{2013}' => 500,
        '\u{2014}' | '\u{2015}' => 1_000,
        '\u{2016}' => 551,
        '\u{2017}' => 411,
        '\u{2018}' | '\u{2019}' | '\u{201B}' => 175,
        '\u{201A}' => 250,
        '\u{201C}' | '\u{201D}' => 359,
        '\u{201E}' => 416,
        '\u{2020}' | '\u{2021}' => 512,
        '\u{2022}' => 376,
        '\u{2026}' => 791,
        _ => return None,
    })
}

fn is_zero_advance_modifier(character: char) -> bool {
    matches!(
        character as u32,
        0x0300..=0x036F
            | 0x0600..=0x0605
            | 0x061C
            | 0x06DD
            | 0x070F
            | 0x0890..=0x0891
            | 0x08E2
            | 0x1AB0..=0x1AFF
            | 0x180B..=0x180E
            | 0x1DC0..=0x1DFF
            | 0x200B..=0x200F
            | 0x20D0..=0x20FF
            | 0x202A..=0x202E
            | 0x2060..=0x206F
            | 0xFE00..=0xFE0F
            | 0xFE20..=0xFE2F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0xE0020..=0xE007F
            | 0xE0100..=0xE01EF
    )
}

fn has_verified_chinese_fullwidth_advance(character: char) -> bool {
    matches!(
        character as u32,
        0x3000..=0x303F
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xFE10..=0xFE1F
            | 0xFE30..=0xFE6F
            | 0xFF01..=0xFF60
            | 0xFFE0..=0xFFE6
    )
}

fn is_explicit_line_break(grapheme: &str) -> bool {
    grapheme.chars().any(|character| {
        matches!(
            character,
            '\n' | '\r' | '\u{000B}' | '\u{000C}' | '\u{0085}' | '\u{2028}' | '\u{2029}'
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{
        CHATBOX_MAX_UTF16_UNITS, ChatboxLayoutError, fits_chatbox_width, grapheme_advance_units,
        is_break_space_grapheme, paginate_completed,
    };
    use std::collections::HashSet;
    use unicode_segmentation::UnicodeSegmentation;

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
        let arabic_pages =
            paginate_completed(&arabic_input).map_err(|error| format!("{error:?}"))?;
        assert_eq!(arabic_pages, vec![ARABIC_9.to_string(), "م".to_string()]);

        let wide_punctuation_input = format!("{WIDE_PUNCTUATION_9}⸻");
        let wide_punctuation_pages =
            paginate_completed(&wide_punctuation_input).map_err(|error| format!("{error:?}"))?;
        assert_eq!(
            wide_punctuation_pages,
            vec![WIDE_PUNCTUATION_9.to_string(), "⸻".to_string()]
        );

        let standalone_skin_tone_input = format!("{STANDALONE_SKIN_TONE_9} 🏽 ");
        let standalone_skin_tone_pages = paginate_completed(&standalone_skin_tone_input)
            .map_err(|error| format!("{error:?}"))?;
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
}
