//! Build-scoped inputs for the conservative Chatbox layout model.
//!
//! The values here come from the verified VRChat 2026.3.1 client resources or
//! from the hash-pinned primary raw font. They are model inputs, not runtime
//! glyph provenance. There is deliberately one private concrete model: a
//! selectable profile seam is not justified until a second client behavior is
//! observed.

const FONT_UNITS_PER_EM: u32 = 1_000;
const FONT_SIZE_LOCAL_UNITS: u32 = 18;
const CHATBOX_WIDTH_LOCAL_UNITS: u32 = 280;
pub(super) const MAX_GRAPHEME_ADVANCE_UNITS: u32 =
    CHATBOX_WIDTH_LOCAL_UNITS * FONT_UNITS_PER_EM / FONT_SIZE_LOCAL_UNITS;

// TextMeshPro's extracted `Leading Characters` table: prefer not to leave one
// of these characters at the end of a wrapped line.
pub(super) const TMP_LEADING_CHARACTERS: &str =
    r##"([｛〔〈《「『【〘〖〝‘“｟«$—…‥〳〴〵\［（{£¥"々〇〉》」＄｠￥￦ #"##;
// TextMeshPro's extracted `Following Characters` table: prefer not to start a
// wrapped line with one of these characters.
pub(super) const TMP_FOLLOWING_CHARACTERS: &str = r##")]｝〕〉》」』】〙〗〟’”｠»ヽヾーァィゥェォッャュョヮヵヶぁぃぅぇぉっゃゅょゎゕゖㇰㇱㇲㇳㇴㇵㇶㇷㇸㇹㇺㇻㇼㇽㇾㇿ々〻‐゠–〜?!‼⁇⁈⁉・、%,.:;。！？］）：；＝}¢°"†‡℃〆％，．"##;

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
pub(super) const POSITIVE_KERNING_PAIRS: [(char, char, u16); 105] = [
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

pub(super) fn fits_chatbox_width(advance_units: u32) -> bool {
    advance_units * FONT_SIZE_LOCAL_UNITS <= CHATBOX_WIDTH_LOCAL_UNITS * FONT_UNITS_PER_EM
}

pub(super) fn positive_kerning_adjustment(left: char, right: char) -> u32 {
    POSITIVE_KERNING_PAIRS
        .binary_search_by_key(&(left, right), |&(pair_left, pair_right, _)| {
            (pair_left, pair_right)
        })
        .map_or(0, |index| u32::from(POSITIVE_KERNING_PAIRS[index].2))
}

pub(super) fn measurable_kerning_character(grapheme: &str) -> Option<char> {
    if requires_conservative_sequence_width(grapheme) {
        return None;
    }

    let mut base = None;
    for character in grapheme.chars() {
        if has_modeled_zero_advance(character) {
            continue;
        }
        if base.is_some() || !has_modeled_primary_font_advance(character) {
            return None;
        }
        base = Some(character);
    }
    base
}

fn has_modeled_primary_font_advance(character: char) -> bool {
    (' '..='~').contains(&character)
        || ('\u{00A0}'..='\u{00FF}').contains(&character)
        || modeled_noto_punctuation_advance(character).is_some()
}

pub(super) fn grapheme_advance_units(grapheme: &str) -> u32 {
    if requires_conservative_sequence_width(grapheme) {
        return MAX_GRAPHEME_ADVANCE_UNITS;
    }

    grapheme
        .chars()
        .map(character_advance_units)
        .fold(0, u32::saturating_add)
        .min(MAX_GRAPHEME_ADVANCE_UNITS)
}

pub(super) fn requires_conservative_sequence_width(grapheme: &str) -> bool {
    grapheme.chars().any(is_complex_sequence_marker)
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

fn character_advance_units(character: char) -> u32 {
    if (' '..='~').contains(&character) {
        return u32::from(BASIC_LATIN_ADVANCES[character as usize - ' ' as usize]);
    }

    if ('\u{00A0}'..='\u{00FF}').contains(&character) {
        return u32::from(LATIN_1_ADVANCES[character as usize - 0x00A0]);
    }

    if let Some(advance) = modeled_noto_punctuation_advance(character) {
        return advance;
    }

    if character == '\t' {
        return u32::from(BASIC_LATIN_ADVANCES[0]) * 4;
    }

    if character.is_control() || has_modeled_zero_advance(character) {
        return 0;
    }

    if uses_modeled_cjk_fullwidth_advance(character) {
        // NotoSansCJK-JP-Regular uses a 1000-unit advance for the covered
        // Chinese ideographs and full-width punctuation.
        return FONT_UNITS_PER_EM;
    }

    // Unsupported graphemes reserve a whole line. A generic 1000-unit fallback
    // is unsafe because some Noto Sans glyphs are substantially wider than one
    // em; the full-line reservation keeps best-effort pagination conservative.
    MAX_GRAPHEME_ADVANCE_UNITS
}

fn modeled_noto_punctuation_advance(character: char) -> Option<u32> {
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

pub(super) fn has_modeled_zero_advance(character: char) -> bool {
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

fn uses_modeled_cjk_fullwidth_advance(character: char) -> bool {
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
