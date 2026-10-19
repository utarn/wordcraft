//! Thai proofing from the bundled PyThaiNLP word list (`assets/spelling/th-words.txt`,
//! CC0-1.0).
//!
//! Thai is written without spaces, so spelling is checked word by word: each Thai run is
//! segmented with [`wordcraft_thai::segments`] (the ICU dictionary FST), and every segment is
//! looked up in the bundled list. A misspelled segment is marked over exactly its byte range,
//! never the whole sentence and never syllable by syllable. Maiyamok (ๆ) and paiyannoi (ฯ) stay
//! with the word before them; a compound that isn't a list entry is accepted when it can be
//! split into known words. Single-grapheme segments, Thai digits, punctuation and other scripts
//! are skipped, so Latin text inside a Thai run is never checked with English rules.
//!
//! The list is deflated at build time (`build.rs`) and decompressed once, within a 64 MiB cap.
//! Entries that are empty, contain whitespace, exceed 80 codepoints or contain a non-Thai
//! character are rejected; a payload with no usable entries turns Thai spelling off (no marks)
//! instead of failing.

use std::sync::OnceLock;

use wordcraft_thai::Segment;

use crate::{Issue, IssueKind, ProofLang};

/// The compressed PyThaiNLP list, written by `build.rs`.
static COMPRESSED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/th-words.bin"));

/// Largest decompressed list accepted (the real one is ~1.5 MB; this caps a corrupt payload).
const MAX_DECOMPRESSED: usize = 64 << 20;
/// Most entries kept. The real list has ~62k.
const MAX_ENTRIES: usize = 500_000;
/// Longest entry kept, in codepoints.
pub(crate) const MAX_WORD_CODEPOINTS: usize = 80;
/// Longest word suggestions are offered for, in codepoints.
const MAX_SUGGEST_WORD_CHARS: usize = 40;
/// Most suggestions ever returned, whatever the caller asks for.
const MAX_SUGGESTIONS: usize = 10;
/// Longest segment tried as a compound, in codepoints (bounds the split search).
const MAX_COMPOUND_CODEPOINTS: usize = 64;
/// Edit distance accepted for a suggestion.
const MAX_SUGGEST_DISTANCE: usize = 2;

/// A sorted, duplicate-free Thai word list.
#[derive(Clone, Debug, Default)]
pub struct Dictionary {
    words: Vec<String>,
}

impl Dictionary {
    /// Build a dictionary from a word-list text (one entry per line). Empty, spaced, over-long
    /// and non-Thai entries are rejected; at most [`MAX_ENTRIES`] are kept. `None` when no
    /// entry survives.
    pub(crate) fn from_text(text: &str) -> Option<Dictionary> {
        let mut words: Vec<String> = text.lines().take(MAX_ENTRIES.saturating_add(1)).filter(|line| valid_entry(line)).map(str::to_string).collect();
        words.sort_unstable();
        words.dedup();
        (!words.is_empty()).then_some(Dictionary { words })
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.words.len()
    }

    pub(crate) fn contains(&self, word: &str) -> bool {
        self.words.binary_search_by(|w| w.as_str().cmp(word)).is_ok()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &str> + '_ {
        self.words.iter().map(String::as_str)
    }
}

/// An entry must be one non-empty run of Thai word characters, at most [`MAX_WORD_CODEPOINTS`].
fn valid_entry(line: &str) -> bool {
    let line = line.strip_prefix('\u{feff}').unwrap_or(line);
    !line.is_empty()
        && line.chars().count() <= MAX_WORD_CODEPOINTS
        && !line.chars().any(char::is_whitespace)
        && line.chars().all(wordcraft_thai::is_thai_word_char)
        && line.chars().any(is_thai_letter)
}

/// Is `c` a Thai letter (a consonant or vowel, not a digit or sign)?
fn is_thai_letter(c: char) -> bool {
    matches!(u32::from(c), 0x0E01..=0x0E3A | 0x0E40..=0x0E4E)
}

/// A combining mark: part of the grapheme cluster before it.
fn is_combining(c: char) -> bool {
    matches!(u32::from(c), 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E)
}

/// Number of grapheme clusters in a Thai word (approximate: a cluster starts at a
/// non-combining character).
fn graphemes(word: &str) -> usize {
    word.chars().filter(|&c| !is_combining(c)).count()
}

/// The dictionary once, or `None` if the bundled payload is corrupt or has no usable entries.
pub fn dictionary() -> Option<&'static Dictionary> {
    static DICT: OnceLock<Option<Dictionary>> = OnceLock::new();
    DICT.get_or_init(|| {
        let raw = miniz_oxide::inflate::decompress_to_vec_with_limit(COMPRESSED, MAX_DECOMPRESSED).ok()?;
        let text = std::str::from_utf8(&raw).ok()?;
        let d = Dictionary::from_text(text);
        if d.is_none() {
            log::warn!("Thai spelling: the bundled word list has no usable entries");
        }
        d
    })
    .as_ref()
}

/// Is a Thai spelling dictionary available? When it isn't, Thai text gets no spelling marks.
pub fn has_dictionary() -> bool {
    dictionary().is_some()
}

/// Strip the signs that attach to the word before them: maiyamok (ๆ) and paiyannoi (ฯ).
fn base_word(word: &str) -> &str {
    word.trim_end_matches(['\u{0E46}', '\u{0E2F}'])
}

/// Is `word` spelled correctly in Thai? Digits, single graphemes, non-Thai scripts and every
/// word when no dictionary is available count as correct (no marks).
pub fn is_correct(word: &str) -> bool {
    is_correct_with(dictionary(), word)
}

pub(crate) fn is_correct_with(dict: Option<&Dictionary>, word: &str) -> bool {
    let Some(d) = dict else { return true };
    if graphemes(word) <= 1 || !word.chars().any(is_thai_letter) {
        return true;
    }
    known(d, word)
}

/// Is one word in the bundled list, the user dictionary, or a known compound? A trailing
/// maiyamok/paiyannoi is stripped only when the word as written is not itself an entry, so
/// abbreviations such as `ฯลฯ` stay correct.
fn known(d: &Dictionary, w: &str) -> bool {
    if d.contains(w) || crate::in_user_dictionary(w) || is_compound(d, w) {
        return true;
    }
    let base = base_word(w);
    base != w && !base.is_empty() && (d.contains(base) || crate::in_user_dictionary(base) || is_compound(d, base))
}

/// Can `w` be split into two or more words that are each in the list (a compound written solid)?
/// Bounded by [`MAX_COMPOUND_CODEPOINTS`] and only over grapheme boundaries.
fn is_compound(d: &Dictionary, w: &str) -> bool {
    if w.chars().count() > MAX_COMPOUND_CODEPOINTS || graphemes(w) < 2 {
        return false;
    }
    // Byte offsets where a grapheme cluster starts, plus the end.
    let mut bounds: Vec<usize> = w.char_indices().filter(|(_, c)| !is_combining(*c)).map(|(i, _)| i).collect();
    bounds.push(w.len());
    let parts = bounds.len().saturating_sub(1);
    if parts < 2 {
        return false;
    }
    // Fewest known words covering each prefix; the whole word needs at least two.
    let mut cover = vec![usize::MAX; parts.saturating_add(1)];
    if let Some(c) = cover.first_mut() {
        *c = 0;
    }
    for i in 0..parts {
        let Some(&before) = cover.get(i) else { continue };
        if before == usize::MAX {
            continue;
        }
        for (j, &end) in bounds.iter().enumerate().skip(i + 1) {
            let (Some(a), Some(_)) = (bounds.get(i), bounds.get(j)) else { continue };
            let Some(sub) = w.get(*a..end) else { continue };
            if sub.chars().count() > MAX_WORD_CODEPOINTS {
                continue;
            }
            if d.contains(sub) || crate::in_user_dictionary(sub) {
                let next = before.saturating_add(1);
                if let Some(c) = cover.get_mut(j)
                    && next < *c
                {
                    *c = next;
                }
            }
        }
    }
    cover.get(parts).is_some_and(|&n| n != usize::MAX && n >= 2)
}

/// The spelling issue of one segment, if it is misspelled. Skipped segments (single graphemes,
/// digits, punctuation, other scripts) and a missing dictionary yield nothing.
pub(crate) fn segment_issue(text: &str, segment: Segment, dict: Option<&Dictionary>) -> Option<Issue> {
    let d = dict?;
    let word = text.get(segment.start..segment.end)?;
    if !word.chars().any(is_thai_letter) || graphemes(word) <= 1 || known(d, word) {
        return None;
    }
    Some(Issue {
        start: segment.start,
        end: segment.end,
        kind: IssueKind::Spelling,
        message: "อาจสะกดผิด".into(),
        suggestions: Vec::new(),
        lang: ProofLang::Th,
    })
}

/// Dictionary-driven suggestions for a misspelled Thai word (best first, at most `max`, and
/// never more than [`MAX_SUGGESTIONS`]).
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    let Some(d) = dictionary() else { return Vec::new() };
    suggest_with(d, word, max)
}

pub(crate) fn suggest_with(d: &Dictionary, word: &str, max: usize) -> Vec<String> {
    let word = base_word(word);
    let n = word.chars().count();
    if n == 0 || n > MAX_SUGGEST_WORD_CHARS || !word.chars().any(is_thai_letter) {
        return Vec::new();
    }
    let target: Vec<char> = word.chars().collect();
    let mut scored: Vec<(usize, usize, String)> = Vec::new();
    for candidate in d.iter() {
        let wn = candidate.chars().count();
        if wn + MAX_SUGGEST_DISTANCE < n || wn > n + MAX_SUGGEST_DISTANCE {
            continue;
        }
        let dist = crate::damerau(&target, candidate);
        if dist <= MAX_SUGGEST_DISTANCE {
            scored.push((dist, wn.abs_diff(n), candidate.to_string()));
        }
    }
    scored.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));
    scored.into_iter().take(max.min(MAX_SUGGESTIONS)).map(|(_, _, w)| w).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_list_loads() {
        let d = dictionary().expect("the bundled CC0 list decompresses");
        assert!(d.len() > 50_000, "{}", d.len());
        for w in ["ภาษา", "ไทย", "สวัสดี", "น้ำแข็ง"] {
            assert!(d.contains(w), "{w}");
        }
        assert!(!d.contains("ไทน"));
    }

    #[test]
    fn rejects_empty_spaced_overlong_and_foreign_entries() {
        let over = "ก".repeat(MAX_WORD_CODEPOINTS + 1);
        let text = format!("\n   \nภาษา\nภาษา ไทย\n{over}\nabc\n๑๒๓\nดี\nproto\n");
        let d = Dictionary::from_text(&text).expect("one valid entry remains");
        assert_eq!(d.len(), 2, "{:?}", d.iter().collect::<Vec<_>>());
        assert!(d.contains("ภาษา") && d.contains("ดี"));
        for bad in ["", "   ", "ภาษา ไทย", over.as_str(), "abc", "๑๒๓"] {
            assert!(!d.contains(bad), "{bad:?}");
        }
    }

    #[test]
    fn no_usable_entries_means_no_dictionary() {
        assert!(Dictionary::from_text("").is_none());
        assert!(Dictionary::from_text("  \n\nabc\n").is_none());
        assert!(Dictionary::from_text(&"ก".repeat(MAX_WORD_CODEPOINTS + 1)).is_none());
        // With no dictionary every word is accepted: Thai marks are off, not everything marked.
        assert!(is_correct_with(None, "ไทน"));
        assert!(suggest_with(&Dictionary::default(), "ไทน", 5).is_empty());
    }

    #[test]
    fn entries_are_capped() {
        let many: String = "ก\n".repeat(MAX_ENTRIES.saturating_add(50));
        let d = Dictionary::from_text(&many).expect("entries");
        assert!(d.len() <= MAX_ENTRIES);
    }

    #[test]
    fn base_words_drop_maiyamok_and_paiyannoi() {
        assert_eq!(base_word("ดีๆ"), "ดี");
        assert_eq!(base_word("ดีๆๆ"), "ดี");
        assert_eq!(base_word("กรุงเทพฯ"), "กรุงเทพ");
        assert_eq!(base_word("ดี"), "ดี");
    }

    #[test]
    fn compounds_split_into_known_words() {
        let d = Dictionary::from_text("ประเทศ\nไทย\nน้ำ\nแข็ง\n").expect("dictionary");
        assert!(is_correct_with(Some(&d), "ประเทศไทย"));
        assert!(is_correct_with(Some(&d), "น้ำแข็ง"));
        assert!(is_correct_with(Some(&d), "ไทย"));
        assert!(!is_correct_with(Some(&d), "ไทน"));
        assert!(!is_correct_with(Some(&d), "ประเทศไทยไทน"));
    }
}
