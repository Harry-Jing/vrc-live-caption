//! Pure text layout for VRChat Chatbox Completed pages and Live viewports.
//!
//! The module has no runtime, pacing, OSC, or queue dependencies. Before any
//! measurement it applies the product control policy: verified line separators
//! and Unicode normalization are preserved, while bare CR, NEL, and form feed
//! become one ASCII space each. It then simulates VRChat's fixed 280-unit
//! TextMeshPro width, nine visible lines, and conservative 144 UTF-16 input
//! budget. Completed layout returns every prepared page in order; Live layout
//! returns one safe viewport retaining the newest prepared text. Soft wraps
//! choose boundaries but are not inserted into returned text. Unsupported
//! Unicode graphemes conservatively reserve a whole line. Every returned page
//! or viewport is revalidated from start-of-text context.

use std::borrow::Cow;
use std::collections::HashMap;
use unicode_linebreak::{BreakOpportunity, linebreaks};
use unicode_segmentation::UnicodeSegmentation;

pub(crate) const CHATBOX_MAX_UTF16_UNITS: usize = 144;
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

// Positive GPOS `kern` xAdvance pairs extracted from the hash-pinned raw
// NotoSans-Regular font whose SHA-256 is
// 6b04c8dd65af6b73eb4279472ed1580b29102d6496a377340e80a40cdb3b22c9.
// This is a source-derived conservative model, not proof that a VRChat client
// selected either these glyphs or these pairs at runtime. Negative adjustments
// are deliberately omitted: applying them could make the prediction narrower
// than the rendered text. The table is ordered by Unicode scalar pair.
// Keep the extracted pair grouping stable so future table updates stay reviewable.
#[rustfmt::skip]
const POSITIVE_KERNING_PAIRS: [(char, char, u16); 105] = [
    ('"', 'T', 20), ('"', 'V', 20), ('"', 'W', 20), ('"', 'Y', 10), ('"', 'Ý', 10),
    ('\'', 'T', 20), ('\'', 'V', 20), ('\'', 'W', 20), ('\'', 'Y', 10), ('\'', 'Ý', 10),
    ('(', 'J', 90), ('(', 'j', 40),
    ('A', 'J', 50),
    ('E', 'J', 60),
    ('F', ')', 20), ('F', '?', 20), ('F', ']', 20), ('F', '}', 20),
    ('T', '?', 20), ('T', 'T', 20),
    ('V', '?', 20),
    ('W', '?', 20),
    ('Y', '?', 20),
    ('[', 'J', 90), ('[', 'j', 40),
    ('c', '"', 20), ('c', '\'', 20), ('c', '’', 20), ('c', '”', 20),
    ('f', '"', 60), ('f', '\'', 60), ('f', ')', 40), ('f', ']', 40), ('f', '}', 40),
    ('f', '’', 60), ('f', '”', 60),
    ('r', '"', 40), ('r', '\'', 40), ('r', '’', 40), ('r', '”', 40),
    ('t', '"', 20), ('t', '\'', 20), ('t', '’', 20), ('t', '”', 20),
    ('v', '"', 40), ('v', '\'', 40), ('v', '?', 20), ('v', '’', 40), ('v', '”', 40),
    ('w', '"', 40), ('w', '\'', 40), ('w', '?', 20), ('w', '’', 40), ('w', '”', 40),
    ('y', '"', 40), ('y', '\'', 40), ('y', '?', 20), ('y', '’', 40), ('y', '”', 40),
    ('{', 'J', 90), ('{', 'j', 40),
    ('¡', 'J', 50),
    ('¿', 'J', 100),
    ('À', 'J', 50), ('Á', 'J', 50), ('Â', 'J', 50),
    ('Ã', 'J', 50), ('Ä', 'J', 50), ('Å', 'J', 50),
    ('Æ', 'J', 60),
    ('È', 'J', 60), ('É', 'J', 60), ('Ê', 'J', 60), ('Ë', 'J', 60),
    ('Ý', '?', 20),
    ('ý', '"', 40), ('ý', '\'', 40), ('ý', '?', 20),
    ('ý', '’', 40), ('ý', '”', 40),
    ('ÿ', '"', 40), ('ÿ', '\'', 40), ('ÿ', '?', 20),
    ('ÿ', '’', 40), ('ÿ', '”', 40),
    ('‘', 'T', 20), ('‘', 'V', 20), ('‘', 'W', 20),
    ('‘', 'Y', 10), ('‘', 'Ý', 10),
    ('’', 'T', 20), ('’', 'V', 20), ('’', 'W', 20),
    ('’', 'Y', 10), ('’', 'Ý', 10),
    ('“', 'T', 20), ('“', 'V', 20), ('“', 'W', 20),
    ('“', 'Y', 10), ('“', 'Ý', 10),
    ('”', 'T', 20), ('”', 'V', 20), ('”', 'W', 20),
    ('”', 'Y', 10), ('”', 'Ý', 10),
];

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ChatboxLayoutError {
    GraphemeExceedsInputBudget { utf16_units: usize },
    RequiresPagination { page_count: usize },
}

/// Exact text that has passed the Chatbox preparation and standalone-layout
/// invariants. The transport can inspect it but cannot construct or mutate it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreparedChatboxText(String);

impl PreparedChatboxText {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// Read-only prediction of how one prepared Chatbox payload lays out.
///
/// Break offsets are UTF-16 boundaries at which the following rendered logical
/// line starts. Explicit offsets therefore include the complete separator
/// (CRLF is one break at the boundary after both code units); a terminal
/// separator adds neither a new visible row nor an offset. Visible lines are
/// capped at VRChat's verified nine-line limit while logical lines retain the
/// full model result.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg(test)]
pub(crate) struct ChatboxLayoutTrace {
    logical_line_count: usize,
    visible_line_count: usize,
    soft_break_utf16_offsets: Vec<usize>,
    explicit_break_utf16_offsets: Vec<usize>,
    clipped: bool,
}

#[cfg(test)]
impl ChatboxLayoutTrace {
    pub(crate) fn logical_line_count(&self) -> usize {
        self.logical_line_count
    }

    pub(crate) fn visible_line_count(&self) -> usize {
        self.visible_line_count
    }

    pub(crate) fn soft_break_utf16_offsets(&self) -> &[usize] {
        &self.soft_break_utf16_offsets
    }

    pub(crate) fn explicit_break_utf16_offsets(&self) -> &[usize] {
        &self.explicit_break_utf16_offsets
    }

    pub(crate) fn clipped(&self) -> bool {
        self.clipped
    }
}

/// Applies the same preparation and layout model as publication, but returns
/// observation data without selecting, mutating, or sending a payload.
#[cfg(test)]
pub(crate) fn trace_layout(text: &str) -> Result<ChatboxLayoutTrace, ChatboxLayoutError> {
    let text = prepare_source_text(text);
    Ok(LayoutText::new(text.as_ref())?.trace())
}

/// Prepares one independently safe Chatbox message without silently selecting
/// one page from a longer input.
pub(crate) fn prepare_single_message(
    text: &str,
) -> Result<Option<PreparedChatboxText>, ChatboxLayoutError> {
    let mut pages = paginate_completed(text)?;
    match pages.len() {
        0 => Ok(None),
        1 => Ok(pages.pop()),
        page_count => Err(ChatboxLayoutError::RequiresPagination { page_count }),
    }
}

/// Returns every safe Completed page in prepared-source order.
///
/// An empty caption has no pages. A single grapheme that is itself larger than
/// VRChat's complete input budget cannot be represented without violating one
/// of the layout invariants, so that pathological input returns an error.
pub(crate) fn paginate_completed(
    text: &str,
) -> Result<Vec<PreparedChatboxText>, ChatboxLayoutError> {
    let text = prepare_source_text(text);
    let text = text.as_ref();
    if text.is_empty() {
        return Ok(Vec::new());
    }

    let prepared = LayoutText::new(text)?;
    let mut pages = Vec::new();
    let mut page_start = 0;

    while page_start < prepared.graphemes.len() {
        let proposed_end = prepared.next_page_end(page_start);
        let (page_end, page) = prepared.standalone_safe_page(page_start, proposed_end)?;
        pages.push(PreparedChatboxText(page));
        page_start = page_end;
    }

    Ok(pages)
}

/// Returns one safe Live viewport that always retains the newest source text.
///
/// Unlike Completed pagination, Live output is a replacement view rather than
/// history. The viewport therefore finds the earliest suffix that is safe when
/// rendered on its own, then advances to the nearest natural line, word, or
/// punctuation boundary when one exists. A single uninterrupted token falls
/// back to the first safe grapheme boundary instead of discarding almost the
/// whole useful view.
pub(crate) fn render_live_viewport(
    text: &str,
) -> Result<Option<PreparedChatboxText>, ChatboxLayoutError> {
    let text = prepare_source_text(text);
    let text = text.as_ref();
    if text.is_empty() {
        return Ok(None);
    }

    // Completed pagination must reject an unrepresentable grapheme because it
    // promises to preserve the whole input. Live is allowed to discard old
    // context, so begin after the newest oversized grapheme when newer content
    // exists. If the newest content itself is unrepresentable there is no safe
    // suffix to publish and the normal error remains useful to diagnostics.
    let mut newest_oversized = None;
    for (start, grapheme) in text.grapheme_indices(true) {
        let utf16_units = grapheme.encode_utf16().count();
        if utf16_units > CHATBOX_MAX_UTF16_UNITS {
            newest_oversized = Some((start + grapheme.len(), utf16_units));
        }
    }
    let text = match newest_oversized {
        Some((end, utf16_units)) if end < text.len() => {
            let Some(suffix) = text.get(end..) else {
                return Err(ChatboxLayoutError::GraphemeExceedsInputBudget { utf16_units });
            };
            if suffix.is_empty() {
                return Err(ChatboxLayoutError::GraphemeExceedsInputBudget { utf16_units });
            }
            suffix
        }
        Some((_, utf16_units)) => {
            return Err(ChatboxLayoutError::GraphemeExceedsInputBudget { utf16_units });
        }
        None => text,
    };

    let prepared = LayoutText::new(text)?;
    let end = prepared.graphemes.len();
    let mut earliest_safe_start = 0;

    // No legal Chatbox message can exceed the UTF-16 input budget. Discarding
    // this definitely-unusable prefix first also bounds the standalone layout
    // checks below to at most one Chatbox-sized candidate.
    while prepared.utf16_units(earliest_safe_start, end) > CHATBOX_MAX_UTF16_UNITS {
        earliest_safe_start += 1;
    }

    while earliest_safe_start < end && !prepared.suffix_is_standalone_safe(earliest_safe_start)? {
        earliest_safe_start += 1;
    }

    // Keep words and punctuation runs intact when advancing the start costs
    // only older context. If one long token has no such boundary, retain the
    // maximal safe grapheme suffix found above.
    let mut preferred_start = earliest_safe_start;
    for start in earliest_safe_start..end {
        if prepared.is_natural_viewport_start(start) && prepared.suffix_is_standalone_safe(start)? {
            preferred_start = start;
            break;
        }
    }

    Ok(Some(PreparedChatboxText(
        prepared.text_between(preferred_start, end),
    )))
}

struct LayoutText<'text> {
    graphemes: Vec<LayoutGrapheme<'text>>,
    utf16_prefix: Vec<usize>,
}

struct LayoutGrapheme<'text> {
    text: &'text str,
    advance_units: u32,
    kerning_character: Option<char>,
    explicit_line_break: bool,
    break_space: bool,
    can_break_after: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LayoutBreakKind {
    Soft,
    Explicit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LayoutBreak {
    next_line_start: usize,
    page_end_at_visible_cap: usize,
    kind: LayoutBreakKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LineScan {
    layout_break: Option<LayoutBreak>,
    latest_legal_break: Option<usize>,
}

impl<'text> LayoutText<'text> {
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
                kerning_character: (!explicit_line_break)
                    .then(|| measurable_kerning_character(grapheme))
                    .flatten(),
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
        let page_budget_end = self.page_budget_end(page_start);
        let budget_clips_text = page_budget_end < self.graphemes.len();
        let mut line_start = page_start;
        let mut line_count = 1;
        let mut last_legal_page_break = None;

        while line_start < page_budget_end {
            let line = self.scan_line(line_start, page_budget_end);
            if let Some(boundary) = line.latest_legal_break {
                last_legal_page_break = Some(boundary);
            }

            let Some(layout_break) = line.layout_break else {
                break;
            };
            if line_count == CHATBOX_MAX_VISIBLE_LINES {
                return match layout_break.kind {
                    LayoutBreakKind::Explicit
                        if layout_break.next_line_start == self.graphemes.len() =>
                    {
                        layout_break.next_line_start
                    }
                    LayoutBreakKind::Explicit => layout_break.page_end_at_visible_cap,
                    LayoutBreakKind::Soft => last_legal_page_break
                        .filter(|boundary| *boundary > page_start)
                        .unwrap_or(layout_break.page_end_at_visible_cap),
                };
            }

            line_count += 1;
            line_start = layout_break.next_line_start;
        }

        if budget_clips_text {
            last_legal_page_break
                .filter(|boundary| *boundary > page_start)
                .unwrap_or(page_budget_end)
        } else {
            self.graphemes.len()
        }
    }

    fn page_budget_end(&self, page_start: usize) -> usize {
        (page_start..self.graphemes.len())
            .take_while(|end| self.utf16_units(page_start, end + 1) <= CHATBOX_MAX_UTF16_UNITS)
            .last()
            .map_or(page_start, |end| end + 1)
    }

    fn scan_line(&self, line_start: usize, scan_end: usize) -> LineScan {
        let mut cursor = line_start;
        let mut line_width = 0_u32;
        let mut last_legal_break = None;

        while cursor < scan_end {
            let grapheme = &self.graphemes[cursor];
            if grapheme.explicit_line_break {
                return LineScan {
                    layout_break: Some(LayoutBreak {
                        next_line_start: cursor + 1,
                        page_end_at_visible_cap: cursor,
                        kind: LayoutBreakKind::Explicit,
                    }),
                    latest_legal_break: Some(cursor + 1),
                };
            }

            let positive_kerning = if cursor == line_start {
                0
            } else {
                self.graphemes[cursor - 1]
                    .kerning_character
                    .zip(grapheme.kerning_character)
                    .map_or(0, |(left, right)| positive_kerning_adjustment(left, right))
            };
            let candidate_width = line_width
                .saturating_add(positive_kerning)
                .saturating_add(grapheme.advance_units);
            if fits_chatbox_width(candidate_width) {
                line_width = candidate_width;
                cursor += 1;
                if grapheme.can_break_after {
                    last_legal_break = Some(cursor);
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

            return LineScan {
                layout_break: Some(LayoutBreak {
                    next_line_start: line_end,
                    page_end_at_visible_cap: line_end,
                    kind: LayoutBreakKind::Soft,
                }),
                latest_legal_break: line_end_is_legal.then_some(line_end),
            };
        }

        LineScan {
            layout_break: None,
            latest_legal_break: last_legal_break,
        }
    }

    #[cfg(test)]
    fn trace(&self) -> ChatboxLayoutTrace {
        if self.graphemes.is_empty() {
            return ChatboxLayoutTrace {
                logical_line_count: 0,
                visible_line_count: 0,
                soft_break_utf16_offsets: Vec::new(),
                explicit_break_utf16_offsets: Vec::new(),
                clipped: false,
            };
        }

        let mut line_start = 0;
        let mut soft_break_utf16_offsets = Vec::new();
        let mut explicit_break_utf16_offsets = Vec::new();
        while line_start < self.graphemes.len() {
            let line = self.scan_line(line_start, self.graphemes.len());
            let Some(layout_break) = line.layout_break else {
                break;
            };
            if layout_break.next_line_start == self.graphemes.len() {
                break;
            }
            let utf16_offset = self.utf16_prefix[layout_break.next_line_start];
            match layout_break.kind {
                LayoutBreakKind::Soft => soft_break_utf16_offsets.push(utf16_offset),
                LayoutBreakKind::Explicit => explicit_break_utf16_offsets.push(utf16_offset),
            }
            line_start = layout_break.next_line_start;
        }

        let logical_line_count =
            1 + soft_break_utf16_offsets.len() + explicit_break_utf16_offsets.len();
        let visible_line_count = logical_line_count.min(CHATBOX_MAX_VISIBLE_LINES);
        ChatboxLayoutTrace {
            logical_line_count,
            visible_line_count,
            soft_break_utf16_offsets,
            explicit_break_utf16_offsets,
            clipped: logical_line_count > visible_line_count,
        }
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
                let standalone = LayoutText::new(&candidate)?;
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

    fn suffix_is_standalone_safe(&self, start: usize) -> Result<bool, ChatboxLayoutError> {
        let candidate = self.text_between(start, self.graphemes.len());
        let standalone = LayoutText::new(&candidate)?;

        Ok(standalone.next_page_end(0) == standalone.graphemes.len())
    }

    fn is_natural_viewport_start(&self, start: usize) -> bool {
        start == 0 || self.graphemes[start - 1].can_break_after
    }

    fn text_between(&self, start: usize, end: usize) -> String {
        self.graphemes[start..end]
            .iter()
            .map(|grapheme| grapheme.text)
            .collect()
    }
}

/// Applies the product-side control policy before any indexing, measurement,
/// pagination, or transmission. CRLF is one verified line break and stays
/// intact; ambiguous standalone controls become one ordinary space each.
fn prepare_source_text(text: &str) -> Cow<'_, str> {
    if !text
        .chars()
        .any(|character| matches!(character, '\r' | '\u{000C}' | '\u{0085}'))
    {
        return Cow::Borrowed(text);
    }

    let mut prepared = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\r' if characters.peek() == Some(&'\n') => prepared.push(character),
            '\r' | '\u{000C}' | '\u{0085}' => prepared.push(' '),
            _ => prepared.push(character),
        }
    }
    Cow::Owned(prepared)
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

fn positive_kerning_adjustment(left: char, right: char) -> u32 {
    POSITIVE_KERNING_PAIRS
        .binary_search_by_key(&(left, right), |&(pair_left, pair_right, _)| {
            (pair_left, pair_right)
        })
        .map_or(0, |index| u32::from(POSITIVE_KERNING_PAIRS[index].2))
}

fn measurable_kerning_character(grapheme: &str) -> Option<char> {
    if requires_conservative_sequence_width(grapheme) {
        return None;
    }

    let mut base = None;
    for character in grapheme.chars() {
        if is_zero_advance_modifier(character) {
            continue;
        }
        if base.is_some() || !has_primary_font_advance(character) {
            return None;
        }
        base = Some(character);
    }
    base
}

fn has_primary_font_advance(character: char) -> bool {
    (' '..='~').contains(&character)
        || ('\u{00A0}'..='\u{00FF}').contains(&character)
        || common_noto_punctuation_advance(character).is_some()
}

fn grapheme_advance_units(grapheme: &str) -> u32 {
    if requires_conservative_sequence_width(grapheme) {
        return MAX_GRAPHEME_ADVANCE_UNITS;
    }

    grapheme
        .chars()
        .map(character_advance_units)
        .fold(0, u32::saturating_add)
        .min(MAX_GRAPHEME_ADVANCE_UNITS)
}

fn requires_conservative_sequence_width(grapheme: &str) -> bool {
    let mut characters = grapheme.chars();
    let Some(base) = characters.next() else {
        return false;
    };
    if base.is_whitespace() && characters.clone().all(is_variation_selector) {
        return false;
    }

    std::iter::once(base)
        .chain(characters)
        .any(is_complex_sequence_marker)
}

fn is_complex_sequence_marker(character: char) -> bool {
    matches!(
        character as u32,
        0x200D
            | 0x20E3
            | 0xFE00..=0xFE0F
            | 0x1F3FB..=0x1F3FF
            | 0xE0020..=0xE007F
            | 0xE0100..=0xE01EF
    )
}

fn is_variation_selector(character: char) -> bool {
    matches!(character as u32, 0xFE00..=0xFE0F | 0xE0100..=0xE01EF)
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
    matches!(
        grapheme,
        "\n" | "\r\n" | "\u{000B}" | "\u{2028}" | "\u{2029}"
    )
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
