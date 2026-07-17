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
#[path = "chatbox_layout_tests.rs"]
mod tests;
