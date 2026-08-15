//! Pure bilingual Completed layout over the verified Chatbox text model.

use super::{
    CHATBOX_MAX_UTF16_UNITS, CHATBOX_MAX_VISIBLE_LINES, ChatboxLayoutError, LayoutText,
    PreparedChatboxText, apply_control_character_policy, prepare_completed_page_from_layout,
    prepare_completed_pages_from_layout, prepare_single_message,
};

const BILINGUAL_SOURCE_BASE_LINES: usize = 4;
const BILINGUAL_TRANSLATION_BASE_LINES: usize = 5;
const BILINGUAL_SEPARATOR: &str = "\n";
const BILINGUAL_SEPARATOR_UTF16_UNITS: usize = 1;

/// One independently safe page for an already-correlated Completed pair.
///
/// Construction and lane composition stay inside layout. Publication can only
/// consume the sealed page as the existing prepared transport capability.
#[must_use = "prepared bilingual pages must be consumed in returned order"]
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
#[allow(
    dead_code,
    reason = "GitHub issue #25 will consume bilingual Completed pages"
)]
pub(crate) struct PreparedBilingualCompletedPage {
    prepared: PreparedChatboxText,
    #[cfg(test)]
    prepared_source: Box<str>,
    #[cfg(test)]
    prepared_translation: Box<str>,
}

#[allow(
    dead_code,
    reason = "GitHub issue #25 will consume bilingual Completed pages"
)]
impl PreparedBilingualCompletedPage {
    #[must_use = "the sealed Chatbox text must be handed to publication"]
    pub(crate) fn into_prepared_text(self) -> PreparedChatboxText {
        self.prepared
    }

    #[cfg(test)]
    fn prepared_source_text(&self) -> &str {
        &self.prepared_source
    }

    #[cfg(test)]
    fn prepared_translation_text(&self) -> &str {
        &self.prepared_translation
    }

    #[cfg(test)]
    fn prepared_text(&self) -> &PreparedChatboxText {
        &self.prepared
    }

    fn source_only(prepared: PreparedChatboxText) -> Self {
        #[cfg(test)]
        let prepared_source = prepared.as_str().into();

        Self {
            prepared,
            #[cfg(test)]
            prepared_source,
            #[cfg(test)]
            prepared_translation: Box::default(),
        }
    }

    fn translation_only(prepared: PreparedChatboxText) -> Self {
        #[cfg(test)]
        let prepared_translation = prepared.as_str().into();

        Self {
            prepared,
            #[cfg(test)]
            prepared_source: Box::default(),
            #[cfg(test)]
            prepared_translation,
        }
    }
}

/// Prepares ordered Chatbox pages for one already-correlated Completed pair.
///
/// Correlation is a caller invariant: layout receives only the exact Source
/// and Translation text selected by the owning publication coordinator.
#[allow(
    dead_code,
    reason = "GitHub issue #25 will call bilingual Completed layout"
)]
pub(crate) fn prepare_bilingual_completed_pages(
    source: &str,
    translation: &str,
) -> Result<Vec<PreparedBilingualCompletedPage>, ChatboxLayoutError> {
    let source = apply_control_character_policy(source);
    let translation = apply_control_character_policy(translation);
    let source = LayoutText::new(source.as_ref())?;
    let translation = LayoutText::new(translation.as_ref())?;
    let mut source_start = 0;
    let mut translation_start = 0;
    let mut pages = Vec::new();

    'shared_pages: while source_start < source.graphemes.len()
        && translation_start < translation.graphemes.len()
    {
        let content_units = CHATBOX_MAX_UTF16_UNITS - BILINGUAL_SEPARATOR_UTF16_UNITS;
        let source_minimum_units = source.utf16_units(source_start, source_start + 1);
        let translation_minimum_units =
            translation.utf16_units(translation_start, translation_start + 1);

        if source_minimum_units + translation_minimum_units > content_units {
            append_source_progress_page(&source, &mut source_start, &mut pages)?;
            continue;
        }

        let (source_lines, translation_lines) = bilingual_line_budgets(
            &source,
            source_start,
            &translation,
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

        let source_reduction = overflow.min(source_units - source_minimum_units);
        source_units -= source_reduction;
        overflow -= source_reduction;
        let translation_reduction = overflow.min(translation_units - translation_minimum_units);
        translation_units -= translation_reduction;

        let mut source_ends =
            fragment_ends_with_limits(&source, source_start, source_lines, source_units)?;
        let mut translation_ends = fragment_ends_with_limits(
            &translation,
            translation_start,
            translation_lines,
            translation_units,
        )?;
        let (Some(&source_end), Some(&translation_end)) =
            (source_ends.last(), translation_ends.last())
        else {
            append_source_progress_page(&source, &mut source_start, &mut pages)?;
            continue;
        };

        let used_source_units = source.utf16_units(source_start, source_end);
        let used_translation_units = translation.utf16_units(translation_start, translation_end);
        let spare_units = content_units - used_source_units - used_translation_units;
        translation_ends = fragment_ends_with_limits(
            &translation,
            translation_start,
            translation_lines,
            used_translation_units + spare_units,
        )?;
        let Some(&translation_end) = translation_ends.last() else {
            append_source_progress_page(&source, &mut source_start, &mut pages)?;
            continue;
        };

        let used_translation_units = translation.utf16_units(translation_start, translation_end);
        let spare_units = content_units - used_source_units - used_translation_units;
        source_ends = fragment_ends_with_limits(
            &source,
            source_start,
            source_lines,
            used_source_units + spare_units,
        )?;
        let Some(&source_end) = source_ends.last() else {
            append_source_progress_page(&source, &mut source_start, &mut pages)?;
            continue;
        };
        debug_assert!(
            source.utf16_units(source_start, source_end)
                + translation.utf16_units(translation_start, translation_end)
                <= content_units,
            "bilingual fragments exceeded the shared input budget"
        );

        loop {
            let (Some(&source_end), Some(&translation_end)) =
                (source_ends.last(), translation_ends.last())
            else {
                append_source_progress_page(&source, &mut source_start, &mut pages)?;
                continue 'shared_pages;
            };

            if let Some(page) = prepare_shared_page(
                &source,
                source_start,
                source_end,
                &translation,
                translation_start,
                translation_end,
            )? {
                pages.push(page);
                source_start = source_end;
                translation_start = translation_end;
                break;
            }

            if source_ends.len() > 1 {
                source_ends.pop();
            } else if translation_ends.len() > 1 {
                translation_ends.pop();
            } else {
                append_source_progress_page(&source, &mut source_start, &mut pages)?;
                continue 'shared_pages;
            }
        }
    }

    pages.extend(
        prepare_completed_pages_from_layout(&source, source_start)?
            .into_iter()
            .map(PreparedBilingualCompletedPage::source_only),
    );
    pages.extend(
        prepare_completed_pages_from_layout(&translation, translation_start)?
            .into_iter()
            .map(PreparedBilingualCompletedPage::translation_only),
    );

    Ok(pages)
}

fn append_source_progress_page(
    source: &LayoutText<'_>,
    source_start: &mut usize,
    pages: &mut Vec<PreparedBilingualCompletedPage>,
) -> Result<(), ChatboxLayoutError> {
    // A shared page always presents Source before Translation. If the two
    // current graphemes or every combined candidate are incompatible, advance
    // Source alone first so the fallback keeps that user-visible order while
    // leaving Translation available for the next shared attempt.
    let (source_end, prepared) =
        prepare_completed_page_from_layout(source, *source_start, *source_start + 1)?;
    pages.push(PreparedBilingualCompletedPage::source_only(prepared));
    *source_start = source_end;
    Ok(())
}

fn bilingual_line_budgets(
    source: &LayoutText<'_>,
    source_start: usize,
    translation: &LayoutText<'_>,
    translation_start: usize,
    content_units: usize,
) -> Result<(usize, usize), ChatboxLayoutError> {
    let source_need = smallest_line_budget_that_fits(
        source,
        source_start,
        BILINGUAL_SOURCE_BASE_LINES,
        content_units,
    )?;
    let translation_need = smallest_line_budget_that_fits(
        translation,
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

fn smallest_line_budget_that_fits(
    lane: &LayoutText<'_>,
    start: usize,
    maximum_lines: usize,
    maximum_utf16_units: usize,
) -> Result<Option<usize>, ChatboxLayoutError> {
    let end = lane.graphemes.len();
    if lane.utf16_units(start, end) > maximum_utf16_units {
        return Ok(None);
    }

    let remaining = lane.text_between(start, end);
    let standalone = LayoutText::new(&remaining)?;
    Ok((1..=maximum_lines).find(|lines| standalone.fits_within_line_budget(*lines)))
}

fn fragment_ends_with_limits(
    lane: &LayoutText<'_>,
    start: usize,
    maximum_lines: usize,
    maximum_utf16_units: usize,
) -> Result<Vec<usize>, ChatboxLayoutError> {
    let mut ends = Vec::new();

    for end in start + 1..=lane.graphemes.len() {
        if lane.utf16_units(start, end) > maximum_utf16_units {
            break;
        }

        let fragment = lane.text_between(start, end);
        let standalone = LayoutText::new(&fragment)?;
        if standalone.fits_within_line_budget(maximum_lines) {
            ends.push(end);
        }
    }

    Ok(ends)
}

fn prepare_shared_page(
    source: &LayoutText<'_>,
    source_start: usize,
    source_end: usize,
    translation: &LayoutText<'_>,
    translation_start: usize,
    translation_end: usize,
) -> Result<Option<PreparedBilingualCompletedPage>, ChatboxLayoutError> {
    let source = source.text_between(source_start, source_end);
    let translation = translation.text_between(translation_start, translation_end);
    let payload = format!("{source}{BILINGUAL_SEPARATOR}{translation}");

    // Reject an exact composition before asking the sealing entrypoint to
    // paginate it merely to report that more than one page was required.
    if !LayoutText::new(&payload)?.fits_within_line_budget(CHATBOX_MAX_VISIBLE_LINES) {
        return Ok(None);
    }

    // Both lanes were prepared before composition, so the ordinary sealed
    // entrypoint's control pass is idempotent and cannot reinterpret a raw
    // Source control together with the synthetic separator.
    match prepare_single_message(&payload) {
        Ok(Some(prepared)) => {
            debug_assert_eq!(prepared.as_str(), payload);
            Ok(Some(PreparedBilingualCompletedPage {
                prepared,
                #[cfg(test)]
                prepared_source: source.into_boxed_str(),
                #[cfg(test)]
                prepared_translation: translation.into_boxed_str(),
            }))
        }
        Ok(None) | Err(ChatboxLayoutError::RequiresPagination { .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
#[path = "bilingual_tests.rs"]
mod tests;
