use super::super::layout::{
    ChatboxLayoutError, prepare_bilingual_completed_pages, prepare_single_message,
};
use super::*;
use crate::error::{AppError, AppResult};

fn layout_error(error: ChatboxLayoutError) -> AppError {
    AppError::runtime(format!(
        "Reading-time test text could not be laid out: {error:?}"
    ))
}

fn page_dwell(text: &str) -> AppResult<Duration> {
    let page = prepare_single_message(text)
        .map_err(layout_error)?
        .ok_or_else(|| AppError::state("Reading-time test text must not be empty."))?;
    Ok(completed_page_dwell(&page))
}

fn characters_per_second(text: &str) -> AppResult<f64> {
    let characters = u32::try_from(text.chars().count())
        .map_err(|_| AppError::state("Reading-time test text was too long."))?;
    Ok(f64::from(characters) / page_dwell(text)?.as_secs_f64())
}

#[test]
fn short_pages_dwell_for_exactly_the_pacing_interval() -> AppResult<()> {
    assert_eq!(MIN_PAGE_DWELL, Duration::from_secs(1));
    for text in ["B", "ok", "next", "   ", "中"] {
        assert_eq!(page_dwell(text)?, MIN_PAGE_DWELL, "{text:?}");
    }
    Ok(())
}

#[test]
fn full_width_text_reads_at_one_em_per_ideograph() -> AppResult<()> {
    assert_eq!(page_dwell(&"中".repeat(10))?, Duration::from_millis(1_200));
    assert_eq!(page_dwell(&"中".repeat(25))?, Duration::from_secs(3));
    assert_eq!(page_dwell(&"中".repeat(50))?, Duration::from_secs(6));
    // Full-width punctuation is modeled at one em as well.
    assert_eq!(
        page_dwell(&"好，".repeat(15))?,
        Duration::from_millis(3_600)
    );
    Ok(())
}

#[test]
fn latin_graphemes_are_charged_their_advance_clamped_to_half_an_em() -> AppResult<()> {
    // `m` is 0.935 em wide and is charged its modeled advance.
    assert_eq!(
        page_dwell(&"m".repeat(40))?,
        Duration::from_micros(4_488_000)
    );
    // `i` is 0.258 em wide, but a narrow letter still takes reading effort,
    // so it is charged half an em.
    assert_eq!(page_dwell(&"i".repeat(40))?, Duration::from_millis(2_400));
    Ok(())
}

#[test]
fn wide_and_unmodeled_graphemes_are_charged_at_most_one_em() -> AppResult<()> {
    // Layout reserves a whole line for each of these graphemes; charged at that
    // width, these nine-line pages would read for the capped dwell instead.
    assert_eq!(page_dwell(&"Ж".repeat(9))?, Duration::from_millis(1_080));
    assert_eq!(
        page_dwell(&"👩\u{200D}💻".repeat(9))?,
        Duration::from_millis(1_080)
    );
    Ok(())
}

#[test]
fn line_separators_and_zero_advance_graphemes_add_no_reading_time() -> AppResult<()> {
    let expected = page_dwell(&"中".repeat(20))?;
    assert_eq!(expected, Duration::from_millis(2_400));
    for separator in ["\n", "\r\n", "\u{2028}", "\u{200B}"] {
        let text = format!("{}{separator}{}", "中".repeat(10), "中".repeat(10));
        assert_eq!(page_dwell(&text)?, expected, "{separator:?}");
    }
    Ok(())
}

#[test]
fn dwell_is_capped_for_long_pages() -> AppResult<()> {
    assert_eq!(page_dwell(&"中".repeat(66))?, Duration::from_millis(7_920));
    assert_eq!(page_dwell(&"中".repeat(67))?, PROVISIONAL_MAX_PAGE_DWELL);
    assert_eq!(page_dwell(&"中".repeat(135))?, PROVISIONAL_MAX_PAGE_DWELL);
    assert_eq!(page_dwell(&"word ".repeat(28))?, PROVISIONAL_MAX_PAGE_DWELL);
    Ok(())
}

#[test]
fn bilingual_page_dwell_counts_both_lanes() -> AppResult<()> {
    let dwells = prepare_bilingual_completed_pages(&"中".repeat(10), &"文".repeat(15))
        .map_err(layout_error)?
        .into_iter()
        .map(|page| completed_page_dwell(&page.into_prepared_text()))
        .collect::<Vec<_>>();

    // The pair shares one page: ten Source and fifteen Translation ideographs
    // are 25 em, read in 3 s.
    assert_eq!(dwells, vec![Duration::from_secs(3)]);
    Ok(())
}

#[test]
fn provisional_rate_matches_latin_and_cjk_subtitle_reading_speeds() -> AppResult<()> {
    let english = "Hello! Can you hear me okay? My microphone was acting up a little earlier \
                   today, sorry about that.";
    let english_rate = characters_per_second(english)?;
    assert!(
        (14.0..=16.0).contains(&english_rate),
        "English prose reads at {english_rate:.1} characters per second"
    );

    let chinese = "我觉得我们应该在活动开始之前先去下一个世界看看。";
    let chinese_rate = characters_per_second(chinese)?;
    assert!(
        (8.0..=9.0).contains(&chinese_rate),
        "Chinese text reads at {chinese_rate:.1} characters per second"
    );
    Ok(())
}
