//! Thai word segmentation and line-break opportunities.
//!
//! Thai is written without spaces between words, so UAX #14 leaves a Thai run unbroken and it
//! runs to the margin. This crate segments Thai into dictionary words with the ICU Thai word
//! list (`assets/thai/thaidict.txt`, Unicode-3.0), compiled by `build.rs` into an FST embedded
//! in the binary (the web build needs no filesystem). Layout uses [`breaks`]; proofing uses
//! [`segments`].
//!
//! Segmentation is a bounded DAG over clusters with backwards shortest-path dynamic programming:
//! a dictionary word costs one token, an unknown cluster costs two, and equal costs favour the
//! longer word. It is deterministic and frequency-free (no corpus weighting). A hostile input
//! cannot run away: a run is segmented in chunks of at most [`MAX_RUN_CODEPOINTS`] codepoints and
//! a word is never looked up past [`MAX_WORD_CODEPOINTS`]; malformed data yields an empty result
//! rather than a panic.
//!
//! Break guards keep Thai typography intact: a line never ends on a leading vowel
//! (U+0E40–U+0E44), never starts on a combining mark (U+0E31, U+0E34–U+0E3A, U+0E47–U+0E4E),
//! and paiyannoi (U+0E2F) and maiyamok (U+0E46) stay attached to the word before them.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::sync::OnceLock;

/// The ICU word list, compiled to an FST by `build.rs`.
static DICT_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/thaidict.fst"));

/// The longest dictionary word considered, in codepoints.
const MAX_WORD_CODEPOINTS: u32 = 55;
/// A run is segmented in chunks no longer than this, in codepoints, so time and memory stay
/// bounded however long the paragraph is.
const MAX_RUN_CODEPOINTS: usize = 100_000;

/// Cost of a dictionary word edge and of an unknown (single-cluster) edge.
const WORD_COST: u64 = 1;
const UNKNOWN_COST: u64 = 2;

/// A word segment: byte offsets into the text passed to [`segments`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub start: usize,
    pub end: usize,
}

/// Whether `c` can be part of a Thai word: letters, vowels and marks, the baht sign, Thai digits
/// and the abbreviation/repetition signs paiyannoi and maiyamok.
pub fn is_thai_word_char(c: char) -> bool {
    matches!(u32::from(c), 0x0E01..=0x0E3A | 0x0E3F | 0x0E40..=0x0E4E | 0x0E50..=0x0E5B)
}

/// A leading vowel (เ แ โ ใ ไ), which belongs to the consonant after it: a line never ends here.
fn is_leading_vowel(c: char) -> bool {
    matches!(u32::from(c), 0x0E40..=0x0E44)
}

/// A combining mark: it belongs to the cluster before it, so a line never starts here.
fn is_combining(c: char) -> bool {
    matches!(u32::from(c), 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E)
}

/// Signs that stay attached to the word before them: paiyannoi and maiyamok.
fn attaches_to_previous(c: char) -> bool {
    matches!(c, '\u{0E2F}' | '\u{0E46}')
}

/// Is `off` a cluster boundary of `text`? Only cluster boundaries may end a word or a line.
fn is_cluster_boundary(text: &str, off: usize) -> bool {
    if off == 0 || off >= text.len() || !text.is_char_boundary(off) {
        return off == 0 || off == text.len();
    }
    text.get(off..).and_then(|t| t.chars().next()).is_some_and(|c| !is_combining(c))
}

/// The embedded dictionary, or `None` if the build produced malformed bytes (never a panic).
fn dictionary() -> Option<&'static fst::raw::Fst<&'static [u8]>> {
    static DICT: OnceLock<Option<fst::raw::Fst<&'static [u8]>>> = OnceLock::new();
    DICT.get_or_init(|| fst::raw::Fst::new(DICT_BYTES).ok()).as_ref()
}

/// One best segmentation of every Thai run in `text`, in ascending byte order. Non-Thai text
/// (spaces, Latin, digits) is not segmented: proofing only checks Thai words.
pub fn segments(text: &str) -> Vec<Segment> {
    let Some(dict) = dictionary() else { return Vec::new() };
    let mut out = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some(&(start, c)) = chars.peek() {
        if !is_thai_word_char(c) {
            chars.next();
            continue;
        }
        chars.next();
        let mut end = start.saturating_add(c.len_utf8());
        while let Some(&(i, c)) = chars.peek() {
            if !is_thai_word_char(c) {
                break;
            }
            end = i.saturating_add(c.len_utf8());
            chars.next();
        }
        if end > start {
            segment_run(text, start, end, dict, &mut out);
        }
    }
    out
}

/// Byte offsets where a Thai line may end, in ascending order. The guards are applied here, so
/// layout can add the result straight to its UAX #14 opportunities. Never includes 0 or
/// `text.len()`, and only offsets strictly inside Thai text.
pub fn breaks(text: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for s in segments(text) {
        let b = s.end;
        if b == 0 || b >= text.len() {
            continue;
        }
        let (Some(prev), Some(next)) = (text.get(..b).and_then(|t| t.chars().next_back()), text.get(b..).and_then(|t| t.chars().next())) else {
            continue;
        };
        // Only within Thai: the UAX #14 opportunities around Latin, digits and spaces stay.
        if !is_thai_word_char(prev) || !is_thai_word_char(next) {
            continue;
        }
        if is_leading_vowel(prev) || is_combining(next) || attaches_to_previous(next) || !is_cluster_boundary(text, b) {
            continue;
        }
        out.push(b);
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Segment one Thai run `text[start..end]`, in bounded chunks, appending to `out`.
fn segment_run(text: &str, start: usize, end: usize, dict: &fst::raw::Fst<&'static [u8]>, out: &mut Vec<Segment>) {
    let Some(run) = text.get(start..end) else { return };
    let mut clusters: Vec<usize> = Vec::new();
    let mut codepoints: Vec<u32> = Vec::new();
    let mut total = 0usize;
    for (i, c) in run.char_indices() {
        let at = start.saturating_add(i);
        if clusters.is_empty() || !is_combining(c) {
            if total >= MAX_RUN_CODEPOINTS && !clusters.is_empty() {
                segment_chunk(text, &clusters, &codepoints, at, dict, out);
                clusters.clear();
                codepoints.clear();
                total = 0;
            }
            clusters.push(at);
            codepoints.push(0);
        }
        if let Some(n) = codepoints.last_mut() {
            *n = n.saturating_add(1);
        }
        total = total.saturating_add(1);
    }
    if !clusters.is_empty() {
        segment_chunk(text, &clusters, &codepoints, end, dict, out);
    }
}

/// Backwards shortest-path DP over the clusters starting at `clusters`. `end` is the byte offset
/// after the last cluster.
fn segment_chunk(text: &str, clusters: &[usize], codepoints: &[u32], end: usize, dict: &fst::raw::Fst<&'static [u8]>, out: &mut Vec<Segment>) {
    let m = clusters.len();
    if m == 0 {
        return;
    }
    let mut cost = vec![0u64; m.saturating_add(1)];
    let mut next = vec![0usize; m.saturating_add(1)];
    for i in (0..m).rev() {
        let mut best = UNKNOWN_COST.saturating_add(cost.get(i + 1).copied().unwrap_or(0));
        let mut best_j = i + 1;
        let mut node = dict.root();
        let mut j = i;
        let mut used = 0u32;
        while j < m {
            let a = clusters.get(j).copied().unwrap_or(end);
            let b = clusters.get(j + 1).copied().unwrap_or(end);
            let Some(bytes) = text.get(a..b).map(str::as_bytes) else { break };
            let mut failed = false;
            for &byte in bytes {
                match node.find_input(byte) {
                    Some(k) => node = dict.node(node.transition(k).addr),
                    None => {
                        failed = true;
                        break;
                    }
                }
            }
            if failed {
                break;
            }
            j = j.saturating_add(1);
            used = used.saturating_add(codepoints.get(j - 1).copied().unwrap_or(1));
            if used > MAX_WORD_CODEPOINTS {
                break;
            }
            if node.is_final() {
                let candidate = WORD_COST.saturating_add(cost.get(j).copied().unwrap_or(0));
                // Equal cost: the longer word wins.
                if candidate < best || (candidate == best && j > best_j) {
                    best = candidate;
                    best_j = j;
                }
            }
        }
        if let (Some(c), Some(n)) = (cost.get_mut(i), next.get_mut(i)) {
            *c = best;
            *n = best_j;
        }
    }
    let mut i = 0usize;
    while i < m {
        let j = next.get(i).copied().unwrap_or(m).clamp(i + 1, m);
        let a = clusters.get(i).copied().unwrap_or(end);
        let b = clusters.get(j).copied().unwrap_or(end);
        if a < b {
            out.push(Segment { start: a, end: b });
        }
        i = j;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<&str> {
        segments(text).iter().filter_map(|s| text.get(s.start..s.end)).collect()
    }

    #[test]
    fn thai_word_chars() {
        for c in ['ก', 'ะ', 'ั', 'ำ', '฿', '๑', 'เ', 'ฯ', 'ๆ'] {
            assert!(is_thai_word_char(c), "{c}");
        }
        for c in [' ', 'A', '1', '\u{0E3B}', '\u{0E4F}', '\u{0E5C}'] {
            assert!(!is_thai_word_char(c), "{c}");
        }
    }

    #[test]
    fn dictionary_words_segment() {
        // "ภาษาไทย" (the Thai language): ภาษา + ไทย.
        assert_eq!(words("ภาษาไทย"), ["ภาษา", "ไทย"]);
        // Pinned fixture, no spaces: every word is a dictionary word and none is per character.
        let text = "ภาษาไทยเป็นภาษาที่สวยงามและมีเสียงวรรณยุกต์ที่ไพเราะ";
        assert_eq!(words(text), ["ภาษา", "ไทย", "เป็น", "ภาษา", "ที่", "สวยงาม", "และ", "มี", "เสียง", "วรรณยุกต์", "ที่", "ไพเราะ"]);
        assert_eq!(breaks(text), vec![12, 21, 33, 45, 54, 72, 81, 87, 102, 129, 138]);
        assert_eq!(words(text).concat(), text);
    }

    #[test]
    fn every_offset_is_a_cluster_boundary() {
        let text = "ภาษาไทย";
        for s in segments(text) {
            assert!(text.is_char_boundary(s.start) && text.is_char_boundary(s.end));
            assert!(is_cluster_boundary(text, s.start) || s.start == 0);
            assert!(is_cluster_boundary(text, s.end) || s.end == text.len());
        }
    }

    #[test]
    fn no_break_after_a_leading_vowel() {
        // Unknown syllables leave a boundary after เ; the guard removes it.
        let text = "กกเก";
        assert!(!breaks(text).contains(&(text.len() - 3)), "{:?}", breaks(text));
        let text = "เกก";
        assert!(!breaks(text).iter().any(|&b| text.get(..b).and_then(|t| t.chars().next_back()) == Some('เ')));
    }

    #[test]
    fn no_break_before_a_combining_mark() {
        for text in ["กก\u{0E31}ก", "กก\u{0E48}ก", "กก\u{0E34}ก"] {
            for b in breaks(text) {
                assert!(text.get(b..).and_then(|t| t.chars().next()).is_some_and(|c| !is_combining(c)), "{text:?} at {b}");
            }
        }
    }

    #[test]
    fn paiyannoi_and_maiyamok_stay_with_the_word_before() {
        for sign in ['\u{0E2F}', '\u{0E46}'] {
            let text = format!("กก{sign}ก");
            let at = text.find(sign).unwrap_or(0);
            assert!(!breaks(&text).contains(&at), "{text:?}: {:?}", breaks(&text));
        }
        // A dictionary abbreviation still breaks after it.
        assert!(breaks("กรุงเทพฯดี").contains(&"กรุงเทพฯ".len()));
    }

    #[test]
    fn breaks_never_zero_or_the_end() {
        let text = "ภาษาไทยเป็นภาษาที่สวยงาม";
        for b in breaks(text) {
            assert!(b > 0 && b < text.len(), "{b}");
        }
    }

    #[test]
    fn non_thai_text_has_no_thai_breaks() {
        let text = "Hello world, 123 and café.";
        assert!(segments(text).is_empty());
        assert!(breaks(text).is_empty());
        assert!(segments("").is_empty());
        assert!(breaks("").is_empty());
    }

    #[test]
    fn mixed_text_keeps_thai_offsets() {
        let text = "ไทย 123 abc ภาษาไทย";
        for b in breaks(text) {
            let prev = text.get(..b).and_then(|t| t.chars().next_back());
            let next = text.get(b..).and_then(|t| t.chars().next());
            assert!(prev.is_some_and(is_thai_word_char) && next.is_some_and(is_thai_word_char), "{b}");
        }
        let second = text.rfind("ภาษาไทย").unwrap_or(0);
        assert!(breaks(text).iter().any(|&b| b > second), "{:?}", breaks(text));
    }

    #[test]
    fn segmentation_is_deterministic() {
        let text = "ภาษาไทยเป็นภาษาที่สวยงามและมีเสียงวรรณยุกต์";
        assert_eq!(segments(text), segments(text));
    }

    #[test]
    fn malformed_and_edge_inputs_do_not_panic() {
        for text in ["", "\u{0E31}", "\u{0E40}", "\u{0E48}\u{0E48}", "ก\u{0E31}\u{0E48}", "ก"] {
            let _ = segments(text);
            let _ = breaks(text);
        }
    }

    #[test]
    fn hostile_million_char_paragraph_is_bounded() {
        // A million codepoints, no spaces: chunked segmentation must finish and never panic.
        let text = "ภาษาไทยเป็นภาษาที่สวยงาม".repeat(50_000);
        assert!(text.chars().count() >= 1_000_000);
        let segs = segments(&text);
        assert!(!segs.is_empty());
        assert!(segs.iter().all(|s| s.start < s.end && s.end <= text.len()));
        assert!(segs.windows(2).all(|w| matches!((w.first(), w.get(1)), (Some(a), Some(b)) if a.end == b.start)));
        let _ = breaks(&text);
    }
}
