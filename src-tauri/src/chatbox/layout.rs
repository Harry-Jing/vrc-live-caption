//! Pure text layout for VRChat Chatbox Completed pages and Live viewports.
//!
//! The module has no runtime, pacing, OSC, or queue dependencies. It simulates
//! VRChat's fixed 280 px TextMeshPro width, nine visible lines, and conservative
//! 144 UTF-16 input budget. Completed layout returns every page in source order;
//! Live layout returns one safe viewport retaining the newest source text. Soft
//! wraps choose boundaries but are not inserted into returned text; explicit
//! source line breaks and other graphemes remain unchanged. Unsupported Unicode
//! graphemes conservatively reserve a whole line. Every returned page or
//! viewport is revalidated from start-of-text context.

use std::collections::HashMap;
use unicode_linebreak::{BreakOpportunity, linebreaks};
use unicode_segmentation::UnicodeSegmentation;

pub(crate) const CHATBOX_MAX_UTF16_UNITS: usize = 144;
const CHATBOX_MAX_VISIBLE_LINES: usize = 9;
const BILINGUAL_SOURCE_BASE_LINES: usize = 4;
const BILINGUAL_TRANSLATION_BASE_LINES: usize = 5;
const BILINGUAL_SEPARATOR: &str = "\n";
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

/// One lossless bilingual page, retaining each lane separately from rendering.
///
/// Keeping lane fragments explicit lets publication preserve exact correlation
/// and lets tests reconstruct both inputs without parsing a presentation
/// separator back out of user-authored text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BilingualCompletedPage {
    source: String,
    translation: String,
}

impl BilingualCompletedPage {
    #[cfg(test)]
    pub(crate) fn source_text(&self) -> &str {
        &self.source
    }

    #[cfg(test)]
    pub(crate) fn translation_text(&self) -> &str {
        &self.translation
    }

    pub(crate) fn rendered_text(&self) -> String {
        match (self.source.is_empty(), self.translation.is_empty()) {
            (false, false) => format!("{}{BILINGUAL_SEPARATOR}{}", self.source, self.translation),
            (false, true) => self.source.clone(),
            (true, false) => self.translation.clone(),
            (true, true) => String::new(),
        }
    }
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

/// Returns deterministic, lossless pages for one exact Completed pair.
///
/// Shared pages reserve four visible lines for Source and five for Translation.
/// If either remaining lane fits in fewer lines, its spare lines are donated to
/// the other lane; otherwise Translation keeps the extra line. The shared
/// UTF-16 budget follows the same 4:5 baseline and donates unused capacity to
/// Translation first. Once one lane is exhausted, the other uses unchanged
/// single-lane Completed pagination.
pub(crate) fn paginate_bilingual_completed(
    source: &str,
    translation: &str,
) -> Result<Vec<BilingualCompletedPage>, ChatboxLayoutError> {
    let source_prepared = PreparedText::new(source)?;
    let translation_prepared = PreparedText::new(translation)?;
    let mut source_start = 0;
    let mut translation_start = 0;
    let mut pages = Vec::new();

    'shared_pages: while source_start < source_prepared.graphemes.len()
        && translation_start < translation_prepared.graphemes.len()
    {
        let separator_units = BILINGUAL_SEPARATOR.encode_utf16().count();
        let content_units = CHATBOX_MAX_UTF16_UNITS - separator_units;
        let source_minimum_units = source_prepared.utf16_units(source_start, source_start + 1);
        let translation_minimum_units =
            translation_prepared.utf16_units(translation_start, translation_start + 1);

        // Two individually representable graphemes can still be too large to
        // share one message with the separator. Preserve both losslessly by
        // advancing only the blocking Source grapheme on a standalone page,
        // then resume shared layout so the remaining lanes can share space.
        if source_minimum_units + translation_minimum_units > content_units {
            let source_end = source_start + 1;
            pages.push(BilingualCompletedPage {
                source: source_prepared.text_between(source_start, source_end),
                translation: String::new(),
            });
            source_start = source_end;
            continue;
        }

        let (source_lines, translation_lines) = bilingual_line_budgets(
            &source_prepared,
            source_start,
            &translation_prepared,
            translation_start,
            content_units,
        )?;
        let source_baseline_units =
            content_units * BILINGUAL_SOURCE_BASE_LINES / CHATBOX_MAX_VISIBLE_LINES;
        let translation_baseline_units = content_units - source_baseline_units;
        let mut source_units = source_baseline_units.max(source_minimum_units);
        let mut translation_units = translation_baseline_units.max(translation_minimum_units);
        let mut overflow = source_units
            .saturating_add(translation_units)
            .saturating_sub(content_units);

        // Translation owns the tie-breaking preference, so shrink Source's
        // discretionary share first when large graphemes exceed the baseline.
        let source_reducible = source_units - source_minimum_units;
        let source_reduction = overflow.min(source_reducible);
        source_units -= source_reduction;
        overflow -= source_reduction;
        let translation_reduction = overflow.min(translation_units - translation_minimum_units);
        translation_units -= translation_reduction;

        let mut source_end =
            source_prepared.fragment_end_with_limits(source_start, source_lines, source_units)?;
        let mut translation_end = translation_prepared.fragment_end_with_limits(
            translation_start,
            translation_lines,
            translation_units,
        )?;
        let mut used_source_units = source_prepared.utf16_units(source_start, source_end);
        let mut used_translation_units =
            translation_prepared.utf16_units(translation_start, translation_end);
        let mut spare_units = content_units - used_source_units - used_translation_units;

        // Width and natural break constraints often leave part of a nominal
        // share unused. Donate that capacity rather than enforcing a fixed
        // split, with Translation receiving the first opportunity.
        translation_end = translation_prepared.fragment_end_with_limits(
            translation_start,
            translation_lines,
            used_translation_units + spare_units,
        )?;
        used_translation_units =
            translation_prepared.utf16_units(translation_start, translation_end);
        spare_units = content_units - used_source_units - used_translation_units;
        source_end = source_prepared.fragment_end_with_limits(
            source_start,
            source_lines,
            used_source_units + spare_units,
        )?;
        used_source_units = source_prepared.utf16_units(source_start, source_end);
        debug_assert!(
            used_source_units + used_translation_units <= content_units,
            "bilingual fragments exceeded the shared input budget"
        );

        // Lane-local budgets do not fully predict the combined result when a
        // fragment contains explicit Unicode line breaks. Revalidate the exact
        // rendered page and retreat at standalone-safe lane boundaries. Source
        // yields first so the deterministic tie-break continues to favor
        // Translation.
        while !bilingual_page_is_standalone_safe(
            &source_prepared,
            source_start,
            source_end,
            &translation_prepared,
            translation_start,
            translation_end,
        )? {
            if source_end > source_start + 1 {
                source_end = source_prepared
                    .standalone_safe_fragment(
                        source_start,
                        source_end - 1,
                        source_lines,
                        source_units,
                    )?
                    .0;
            } else if translation_end > translation_start + 1 {
                translation_end = translation_prepared
                    .standalone_safe_fragment(
                        translation_start,
                        translation_end - 1,
                        translation_lines,
                        translation_units,
                    )?
                    .0;
            } else {
                pages.push(BilingualCompletedPage {
                    source: source_prepared.text_between(source_start, source_end),
                    translation: String::new(),
                });
                source_start = source_end;
                continue 'shared_pages;
            }
        }

        pages.push(BilingualCompletedPage {
            source: source_prepared.text_between(source_start, source_end),
            translation: translation_prepared.text_between(translation_start, translation_end),
        });
        source_start = source_end;
        translation_start = translation_end;
    }

    if source_start < source_prepared.graphemes.len() {
        let remaining = source_prepared.text_between(source_start, source_prepared.graphemes.len());
        pages.extend(paginate_completed(&remaining)?.into_iter().map(|source| {
            BilingualCompletedPage {
                source,
                translation: String::new(),
            }
        }));
    }
    if translation_start < translation_prepared.graphemes.len() {
        let remaining = translation_prepared
            .text_between(translation_start, translation_prepared.graphemes.len());
        pages.extend(
            paginate_completed(&remaining)?
                .into_iter()
                .map(|translation| BilingualCompletedPage {
                    source: String::new(),
                    translation,
                }),
        );
    }

    Ok(pages)
}

fn bilingual_page_is_standalone_safe(
    source: &PreparedText<'_>,
    source_start: usize,
    source_end: usize,
    translation: &PreparedText<'_>,
    translation_start: usize,
    translation_end: usize,
) -> Result<bool, ChatboxLayoutError> {
    let rendered = format!(
        "{}{BILINGUAL_SEPARATOR}{}",
        source.text_between(source_start, source_end),
        translation.text_between(translation_start, translation_end)
    );
    let prepared = PreparedText::new(&rendered)?;

    Ok(prepared.next_page_end(0) == prepared.graphemes.len())
}

fn bilingual_line_budgets(
    source: &PreparedText<'_>,
    source_start: usize,
    translation: &PreparedText<'_>,
    translation_start: usize,
    content_units: usize,
) -> Result<(usize, usize), ChatboxLayoutError> {
    let source_need = source.smallest_line_budget_that_fits(
        source_start,
        BILINGUAL_SOURCE_BASE_LINES,
        content_units,
    )?;
    let translation_need = translation.smallest_line_budget_that_fits(
        translation_start,
        BILINGUAL_TRANSLATION_BASE_LINES,
        content_units,
    )?;

    Ok(match (source_need, translation_need) {
        (Some(source_lines), Some(translation_lines)) => (source_lines, translation_lines),
        (Some(source_lines), None) => (source_lines, CHATBOX_MAX_VISIBLE_LINES - source_lines),
        (None, Some(translation_lines)) => (
            CHATBOX_MAX_VISIBLE_LINES - translation_lines,
            translation_lines,
        ),
        (None, None) => (
            BILINGUAL_SOURCE_BASE_LINES,
            BILINGUAL_TRANSLATION_BASE_LINES,
        ),
    })
}

/// Returns one safe Live viewport that always retains the newest source text.
///
/// Unlike Completed pagination, Live output is a replacement view rather than
/// history. The viewport therefore finds the earliest suffix that is safe when
/// rendered on its own, then advances to the nearest natural line, word, or
/// punctuation boundary when one exists. A single uninterrupted token falls
/// back to the first safe grapheme boundary instead of discarding almost the
/// whole useful view.
pub(crate) fn render_live_viewport(text: &str) -> Result<String, ChatboxLayoutError> {
    if text.is_empty() {
        return Ok(String::new());
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
            let Some(suffix) = text.get(end..).map(str::trim_start) else {
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

    let prepared = PreparedText::new(text)?;
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

    Ok(prepared.text_between(preferred_start, end))
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
        self.next_page_end_with_limits(
            page_start,
            CHATBOX_MAX_VISIBLE_LINES,
            CHATBOX_MAX_UTF16_UNITS,
        )
    }

    fn next_page_end_with_limits(
        &self,
        page_start: usize,
        max_visible_lines: usize,
        max_utf16_units: usize,
    ) -> usize {
        let mut cursor = page_start;
        let mut line_start = page_start;
        let mut line_width = 0;
        let mut line_count = 1;
        let mut last_legal_break = None;
        let mut last_legal_page_break = None;

        while cursor < self.graphemes.len() {
            let grapheme = &self.graphemes[cursor];
            if self.utf16_units(page_start, cursor + 1) > max_utf16_units {
                return last_legal_page_break
                    .filter(|boundary| *boundary > page_start)
                    .unwrap_or(cursor);
            }

            if grapheme.explicit_line_break {
                if line_count == max_visible_lines {
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

            if line_count == max_visible_lines {
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
        self.standalone_safe_fragment(
            page_start,
            proposed_end,
            CHATBOX_MAX_VISIBLE_LINES,
            CHATBOX_MAX_UTF16_UNITS,
        )
    }

    fn standalone_safe_fragment(
        &self,
        page_start: usize,
        proposed_end: usize,
        max_visible_lines: usize,
        max_utf16_units: usize,
    ) -> Result<(usize, String), ChatboxLayoutError> {
        let mut page_end = proposed_end;

        loop {
            let candidate: String = self.graphemes[page_start..page_end]
                .iter()
                .map(|grapheme| grapheme.text)
                .collect();
            let safe_byte_len = {
                let standalone = PreparedText::new(&candidate)?;
                let standalone_end =
                    standalone.next_page_end_with_limits(0, max_visible_lines, max_utf16_units);
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

    fn fragment_end_with_limits(
        &self,
        start: usize,
        max_visible_lines: usize,
        max_utf16_units: usize,
    ) -> Result<usize, ChatboxLayoutError> {
        let proposed_end =
            self.next_page_end_with_limits(start, max_visible_lines, max_utf16_units);
        if proposed_end == start {
            return Ok(start);
        }

        self.standalone_safe_fragment(start, proposed_end, max_visible_lines, max_utf16_units)
            .map(|(end, _)| end)
    }

    fn smallest_line_budget_that_fits(
        &self,
        start: usize,
        maximum: usize,
        max_utf16_units: usize,
    ) -> Result<Option<usize>, ChatboxLayoutError> {
        for lines in 1..=maximum {
            if self.fragment_end_with_limits(start, lines, max_utf16_units)? == self.graphemes.len()
            {
                return Ok(Some(lines));
            }
        }

        Ok(None)
    }

    fn suffix_is_standalone_safe(&self, start: usize) -> Result<bool, ChatboxLayoutError> {
        let candidate = self.text_between(start, self.graphemes.len());
        let standalone = PreparedText::new(&candidate)?;

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
#[path = "layout_tests.rs"]
mod tests;
