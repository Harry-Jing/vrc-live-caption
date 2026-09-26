//! Provisional reading time for Completed Chatbox pages.
//!
//! Every Chatbox message replaces the one before it, so a Completed page must
//! stay visible long enough to be read before the next Completed page replaces
//! it. The shared text pacer only separates send attempts; this policy decides
//! how long one page holds the Chatbox. A page with nothing queued behind it
//! simply stays visible.
//!
//! The measure reuses the build-scoped layout model, so it is deterministic and
//! script-aware without language detection. Each grapheme contributes its
//! modeled advance clamped to between half an em and one em:
//!
//! - the floor charges narrow Latin glyphs, punctuation, and spaces like the
//!   characters that subtitle reading-speed guidance counts;
//! - the ceiling keeps a wide or unmodeled grapheme, which layout reserves as a
//!   whole line, from being read as a full line of text;
//! - explicit line separators and zero-advance graphemes contribute nothing.
//!
//! At 120 ms per em, English prose (about 0.56 em per character after the
//! clamp) reads at about 15 characters per second and full-width CJK text at
//! about 8.3 characters per second, matching common subtitle guidance for Latin
//! and Chinese text. Bilingual and Translation-only pages use the same rule
//! over their full composed content. Every number here is provisional until
//! native VRChat readability validation.

use super::PreparedChatboxText;
use super::text_pacing::CHATBOX_TEXT_ATTEMPT_INTERVAL;
use std::time::Duration;

const MILLI_EMS_PER_EM: u32 = 1_000;
const PROVISIONAL_READING_TIME_PER_EM: Duration = Duration::from_millis(120);
const PROVISIONAL_MIN_GRAPHEME_READING_MILLI_EMS: u32 = MILLI_EMS_PER_EM / 2;
const PROVISIONAL_MAX_GRAPHEME_READING_MILLI_EMS: u32 = MILLI_EMS_PER_EM;

/// A page never dwells for less than one pacing interval, so a short page is
/// replaced exactly as early as the shared pacer alone would allow.
pub(super) const MIN_PAGE_DWELL: Duration = CHATBOX_TEXT_ATTEMPT_INTERVAL;

/// Eight seconds shows a full English page (about 140 characters) at about 17.5
/// characters per second, within common adult subtitle limits, and the Chinese
/// lane of a full en→zh Bilingual page (about 75 characters) at about 9 per
/// second. It also bounds how long one page can hold back sustained speech;
/// denser pages, such as a full 135-character CJK page, therefore read faster
/// than the target rate. Provisional until native readability validation.
pub(super) const PROVISIONAL_MAX_PAGE_DWELL: Duration = Duration::from_secs(8);

/// Returns how long `page` stays visible before a later Completed page may
/// replace it: its reading time, clamped to the pacing floor and the cap.
pub(super) fn completed_page_dwell(page: &PreparedChatboxText) -> Duration {
    let reading_milli_ems = page
        .grapheme_advances_milli_em()
        .filter(|advance| *advance > 0)
        .map(|advance| {
            advance.clamp(
                PROVISIONAL_MIN_GRAPHEME_READING_MILLI_EMS,
                PROVISIONAL_MAX_GRAPHEME_READING_MILLI_EMS,
            )
        })
        .fold(0_u32, u32::saturating_add);
    let reading_time =
        PROVISIONAL_READING_TIME_PER_EM.saturating_mul(reading_milli_ems) / MILLI_EMS_PER_EM;

    reading_time.clamp(MIN_PAGE_DWELL, PROVISIONAL_MAX_PAGE_DWELL)
}

#[cfg(test)]
#[path = "reading_time_tests.rs"]
mod tests;
