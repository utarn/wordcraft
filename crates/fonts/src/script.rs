//! Scripts: itemising text into single-script runs for the shaper, and the fonts preferred when
//! a document's font lacks a complex script (Myanmar, Khmer, the Indic scripts, Thai…).
//!
//! A shaper picks its script-specific engine (the Myanmar, Khmer, Indic or Universal Shaping
//! Engine reordering, e.g. U+1031 ေ drawn before its consonant) from the run's script, so text
//! mixing scripts in one face must be shaped one script at a time.

#![allow(clippy::single_range_in_vec_init)] // one run is a list of runs

use unicode_script::{Script, UnicodeScript};

/// The script of `c` for itemisation: `None` for characters that take the script of the text
/// around them (spaces, punctuation, digits, combining marks, joiners).
fn real_script(c: char) -> Option<Script> {
    match c.script() {
        Script::Common | Script::Inherited | Script::Unknown => None,
        // Japanese and Chinese mix these within a word: one run.
        Script::Hiragana | Script::Katakana | Script::Bopomofo => Some(Script::Han),
        s => Some(s),
    }
}

/// Maximal single-script byte ranges of `text`. Characters of no particular script join the run
/// before them (or, at the start, the first run), so the ranges cover `text` exactly.
pub(crate) fn script_runs(text: &str) -> Vec<std::ops::Range<usize>> {
    if text.is_ascii() {
        return vec![0..text.len()];
    }
    let mut runs = Vec::new();
    let mut cur: Option<Script> = None;
    let mut start = 0;
    for (i, c) in text.char_indices() {
        let Some(s) = real_script(c) else { continue };
        match cur {
            Some(p) if p != s => {
                runs.push(start..i);
                start = i;
                cur = Some(s);
            }
            None => cur = Some(s),
            _ => {}
        }
    }
    runs.push(start..text.len());
    runs
}

/// Scripts whose shaping reorders, stacks or forms clusters (OpenType script-specific engines),
/// so a fallback font must carry OpenType layout tables to draw them properly.
pub fn needs_layout_tables(c: char) -> bool {
    matches!(
        c.script(),
        Script::Myanmar
            | Script::Khmer
            | Script::Devanagari
            | Script::Bengali
            | Script::Gurmukhi
            | Script::Gujarati
            | Script::Oriya
            | Script::Tamil
            | Script::Telugu
            | Script::Kannada
            | Script::Malayalam
            | Script::Sinhala
            | Script::Thai
            | Script::Lao
            | Script::Tibetan
            | Script::Tai_Tham
            | Script::Balinese
            | Script::Javanese
            | Script::Arabic
            | Script::Syriac
    )
}

/// Families preferred, in order, for a character of `c`'s script when the document's font lacks
/// it: the platforms' own fonts and the open Noto/SIL families. Thai returns the same ordered
/// policy Word font substitution uses ([`crate::thai`]), so both paths choose one face. Other
/// scripts' families are ordered as before.
pub fn script_fallbacks(c: char) -> &'static [&'static str] {
    if is_emoji(c) {
        return COLOR_EMOJI;
    }
    match c.script() {
        Script::Myanmar => &["Myanmar Text", "Noto Sans Myanmar", "Padauk", "Pyidaungsu", "Noto Serif Myanmar", "Myanmar Sangam MN", "Myanmar MN"],
        Script::Khmer => &["Leelawadee UI", "Khmer UI", "Noto Sans Khmer", "Noto Serif Khmer", "Khmer OS", "Khmer Sangam MN", "Khmer MN"],
        Script::Devanagari => &["Nirmala UI", "Mangal", "Kohinoor Devanagari", "Noto Sans Devanagari", "Noto Serif Devanagari", "Lohit Devanagari"],
        Script::Bengali => &["Nirmala UI", "Vrinda", "Kohinoor Bangla", "Noto Sans Bengali", "Noto Serif Bengali", "Lohit Bengali"],
        // The same ordered policy Word font substitution uses (#15), so a Thai character falls
        // back to the same face whichever path found it.
        Script::Thai => crate::thai::candidates(crate::thai::Role::Sans, crate::thai::current_platform()),
        Script::Lao => &["Leelawadee UI", "Lao UI", "Noto Sans Lao", "Noto Serif Lao", "Lao Sangam MN"],
        Script::Tamil | Script::Telugu | Script::Kannada | Script::Malayalam | Script::Gujarati | Script::Gurmukhi | Script::Oriya => &["Nirmala UI"],
        _ => &[],
    }
}

/// The platforms' colour emoji fonts. Emoji prefer them over the monochrome symbol fonts that
/// also cover many emoji (Windows' Segoe UI Symbol has ✅, so it drew in black and white).
const COLOR_EMOJI: &[&str] = &["Segoe UI Emoji", "Apple Color Emoji", "Noto Color Emoji"];

/// Characters drawn as emoji by default (Unicode `Emoji_Presentation`): the BMP ones listed in
/// emoji-data.txt, and the pictograph blocks from U+1F000.
// ponytail: all of U+1F000..U+1FAFF counts as emoji (a few text-default pictographs too); switch to the exact emoji-data ranges if one must draw monochrome
pub fn is_emoji(c: char) -> bool {
    matches!(
        u32::from(c),
        0x1F000..=0x1FAFF
            | 0x231A..=0x231B
            | 0x23E9..=0x23EC
            | 0x23F0
            | 0x23F3
            | 0x25FD..=0x25FE
            | 0x2614..=0x2615
            | 0x2648..=0x2653
            | 0x267F
            | 0x2693
            | 0x26A1
            | 0x26AA..=0x26AB
            | 0x26BD..=0x26BE
            | 0x26C4..=0x26C5
            | 0x26CE
            | 0x26D4
            | 0x26EA
            | 0x26F2..=0x26F3
            | 0x26F5
            | 0x26FA
            | 0x26FD
            | 0x2705
            | 0x270A..=0x270B
            | 0x2728
            | 0x274C
            | 0x274E
            | 0x2753..=0x2755
            | 0x2757
            | 0x2795..=0x2797
            | 0x27B0
            | 0x27BF
            | 0x2B1B..=0x2B1C
            | 0x2B50
            | 0x2B55
    )
}

/// How well `family` suits `c` as a fallback: 0 for the first preferred family, then the others
/// in order; then families named after the script ("Noto Sans Tamil"); then everything else.
pub fn fallback_rank(c: char, family: &str) -> usize {
    let list = script_fallbacks(c);
    if let Some(i) = list.iter().position(|n| family.eq_ignore_ascii_case(n)) {
        return i;
    }
    if needs_layout_tables(c) {
        let name = c.script().full_name().replace('_', " ");
        if family.to_ascii_lowercase().contains(&name.to_ascii_lowercase()) {
            return list.len();
        }
    }
    list.len() + 1
}

/// The distinct complex-script characters (Thai, Arabic, the Indic scripts, Myanmar, Khmer…) in
/// `text` that no installed face can draw: the boxes a page would show. Bounded to
/// [`MAX_DIAGNOSTIC_CHARS`] distinct characters and to that many fallback searches, however
/// hostile the text or the installed fonts.
pub fn uncovered_complex(db: &crate::FontDb, text: &str) -> Vec<char> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for c in text.chars() {
        if !needs_layout_tables(c) || c.is_control() || c.is_whitespace() || !seen.insert(c) {
            continue;
        }
        // A placeholder face (`.LastResort`) has no real glyph for anything: it draws a box.
        if db.fallback_for(c, 0).is_none_or(|f| f.is_placeholder()) {
            out.push(c);
            if out.len() >= MAX_DIAGNOSTIC_CHARS {
                break;
            }
        }
    }
    out
}

/// Most distinct uncovered characters a diagnostic reports (bounds font searches).
pub const MAX_DIAGNOSTIC_CHARS: usize = 1024;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_split_by_script_and_neutrals_join_their_neighbours() {
        assert_eq!(script_runs("Hello"), vec![0..5]);
        // "Hi မြန်မာ!" → Latin "Hi ", Myanmar "မြန်မာ!".
        let s = "Hi မြန်မာ!";
        let runs = script_runs(s);
        assert_eq!(runs, vec![0..3, 3..s.len()]);
        // A leading neutral joins the first run; marks never start a run.
        assert_eq!(script_runs("(ကေ)"), vec![0.."(ကေ)".len()]);
        // Japanese kana and kanji stay together.
        assert_eq!(script_runs("日本語のテキスト").len(), 1);
        assert_eq!(script_runs(""), vec![0..0]);
    }

    #[test]
    fn thai_script_fallback_shares_the_word_substitution_policy() {
        use crate::thai::{Role, candidates, current_platform};
        assert_eq!(script_fallbacks('\u{0E19}'), candidates(Role::Sans, current_platform()));
        assert_eq!(fallback_rank('\u{0E19}', "TH Sarabun New"), 0);
        assert!(fallback_rank('\u{0E19}', "TH Sarabun New") < fallback_rank('\u{0E19}', "TH SarabunPSK"));
        // A Thai-capable Latin family ranks in the same table, above an unrelated Latin face.
        assert!(fallback_rank('\u{0E19}', "Tahoma") < fallback_rank('\u{0E19}', "Carlito"));
    }

    #[test]
    fn complex_scripts_prefer_their_own_fonts() {
        assert!(needs_layout_tables('မ') && needs_layout_tables('ក') && needs_layout_tables('क'));
        assert!(!needs_layout_tables('a') && !needs_layout_tables('語'));
        assert_eq!(fallback_rank('မ', "Myanmar Text"), 0);
        assert!(fallback_rank('မ', "Noto Sans Myanmar") < fallback_rank('မ', "Myanmar Sangam MN"));
        assert!(fallback_rank('த', "Noto Sans Tamil") < fallback_rank('த', "Arial"));
        assert_eq!(fallback_rank('a', "Arial"), fallback_rank('a', "Noto Sans"));
    }

    #[test]
    fn emoji_prefer_colour_emoji_fonts() {
        for c in ['✅', '🧠', '⭐', '⌚', '🫶'] {
            assert!(is_emoji(c), "{c}");
            assert!(fallback_rank(c, "Segoe UI Emoji") < fallback_rank(c, "Segoe UI Symbol"), "{c}");
            assert!(fallback_rank(c, "Noto Color Emoji") < fallback_rank(c, "Noto Emoji"), "{c}");
        }
        // Text-style symbols keep the symbol fonts.
        for c in ['✔', '☺', '→', 'a', '1'] {
            assert!(!is_emoji(c), "{c}");
        }
    }

    #[test]
    fn uncovered_complex_reports_the_characters_that_would_be_boxes() {
        // No system fonts and no bundled Thai face: every Thai character is a box, once.
        let db = crate::FontDb::with_font_dirs(Vec::new());
        db.set_system_fallback(false);
        let missing = uncovered_complex(&db, "น้ำ น้ำ ABC");
        assert!(missing.contains(&'\u{0E19}'), "{missing:?}");
        assert!(missing.contains(&'\u{0E33}'), "SARA AM is complex too: {missing:?}");
        assert_eq!(missing.len(), missing.iter().collect::<std::collections::HashSet<_>>().len(), "each character once");
        assert!(!missing.iter().any(|c| c.is_ascii()), "Latin text is not a complex-script box");
        // Latin-only text asks for nothing, and hostile text is bounded.
        assert!(uncovered_complex(&db, "Plain Latin 123").is_empty());
        let long: String = (0..(MAX_DIAGNOSTIC_CHARS + 100)).filter_map(|i| char::from_u32(0x0E00 + i as u32)).collect();
        assert!(uncovered_complex(&db, &long).len() <= MAX_DIAGNOSTIC_CHARS);
    }
}
