//! Thai proofing from an installed Hunspell `th_TH` dictionary or the bundled PyThaiNLP word
//! list (`assets/spelling/th-words.txt`, CC0-1.0).
//!
//! Thai is written without spaces, so spelling is checked word by word: each Thai run is
//! segmented with [`wordcraft_thai::segments`] (the ICU dictionary FST), and every segment is
//! looked up in the active dictionary. A misspelled segment is marked over exactly its byte
//! range, never the whole sentence and never syllable by syllable. Maiyamok (ๆ) and paiyannoi
//! (ฯ) stay with the word before them; a compound that isn't an entry is accepted when it can be
//! split into known words. Single-grapheme segments, Thai digits, punctuation and other scripts
//! are skipped, so Latin text inside a Thai run is never checked with English rules.
//!
//! Dictionary order (the first that loads wins): a user Hunspell folder
//! ([`crate::hunspell::set_dictionary_dir`], `WORDCRAFT_DICTIONARIES`), then the system's
//! `th_TH` pair (read at runtime — the LGPL data is never copied into the tree), then the
//! bundled list. The system dictionary ships as TIS-620; [`crate::hunspell::decode`] reads it
//! and its `REP` pairs drive suggestions. Phrase entries and slash-separated alternatives are
//! sanitized before the affix reader sees them, so neither is mis-parsed as a stem with flags.
//!
//! Both lists are bounded: files are size-capped by [`crate::hunspell`], entries are length- and
//! count-capped here, and the bundled payload is decompressed once within a 64 MiB cap. Empty,
//! malformed or missing dictionaries leave Thai marks off instead of failing.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use wordcraft_thai::Segment;

use crate::affix::AffixDict;
use crate::{Issue, IssueKind, ProofLang};

/// The compressed PyThaiNLP list, written by `build.rs`.
static COMPRESSED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/th-words.bin"));

/// File names of a Thai Hunspell dictionary, most specific first.
const NAMES: &[&str] = &["th_TH", "th-TH", "th"];
/// Largest decompressed list accepted (the real one is ~1.5 MB; this caps a corrupt payload).
const MAX_DECOMPRESSED: usize = 64 << 20;
/// Most entries kept. The real lists have ~62k (bundled) and ~40k (system).
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
/// Most stems scanned for an edit-distance suggestion (bounds a hostile dictionary).
const MAX_SUGGEST_CANDIDATES: usize = 200_000;

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

/// The dictionary a Thai word is checked against.
#[derive(Clone)]
pub(crate) enum Active {
    /// An installed Hunspell `th_TH` pair (user folder or system).
    Hunspell(Arc<AffixDict>),
    /// The bundled CC0 word list.
    Bundled(Arc<Dictionary>),
}

impl Active {
    /// Is `w` spelled as written?
    fn contains(&self, w: &str) -> bool {
        match self {
            Active::Hunspell(d) => d.check(w),
            Active::Bundled(d) => d.contains(w),
        }
    }

    /// Is `w` a form a suggestion may offer (not forbidden or no-suggest)?
    fn suggestible(&self, w: &str) -> bool {
        match self {
            Active::Hunspell(d) => d.can_suggest(w),
            Active::Bundled(d) => d.contains(w),
        }
    }

    /// The Hunspell `REP` table; the bundled list has none.
    fn rep(&self) -> &[(String, String)] {
        match self {
            Active::Hunspell(d) => d.rep(),
            Active::Bundled(_) => &[],
        }
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

/// The bundled list once, or `None` if the payload is corrupt or has no usable entries.
fn bundled() -> Option<Arc<Dictionary>> {
    static BUNDLED: OnceLock<Option<Arc<Dictionary>>> = OnceLock::new();
    BUNDLED
        .get_or_init(|| {
            let raw = miniz_oxide::inflate::decompress_to_vec_with_limit(COMPRESSED, MAX_DECOMPRESSED).ok()?;
            let text = std::str::from_utf8(&raw).ok()?;
            let d = Dictionary::from_text(text);
            if d.is_none() {
                log::warn!("Thai spelling: the bundled word list has no usable entries");
            }
            d.map(Arc::new)
        })
        .clone()
}

/// The installed dictionary: `None` until looked for once (then `Some(None)` when absent, until
/// [`reload_dictionary`]).
static INSTALLED: RwLock<Option<Option<Arc<AffixDict>>>> = RwLock::new(None);

/// The active dictionary: an installed Hunspell pair when one loads, else the bundled list.
/// `None` when neither is usable (Thai spelling is off then).
pub(crate) fn dictionary() -> Option<Active> {
    if let Some(d) = installed() {
        return Some(Active::Hunspell(d));
    }
    bundled().map(Active::Bundled)
}

/// The installed Hunspell dictionary, looked for on first use and cached. A pair that is
/// missing, malformed, empty or over a cap counts as not installed, so the bundled list is used.
fn installed() -> Option<Arc<AffixDict>> {
    if let Some(cached) = INSTALLED.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return cached.clone();
    }
    let loaded = load_any(installed_candidates());
    let mut slot = INSTALLED.write().unwrap_or_else(|e| e.into_inner());
    // Another thread may have loaded it meanwhile: that one wins.
    if slot.is_none() {
        *slot = Some(loaded);
    }
    slot.as_ref().and_then(|cached| cached.clone())
}

/// Forget the installed dictionary, so the next check looks for one again (after the
/// dictionaries folder changed or a dictionary was installed or replaced).
pub fn reload_dictionary() {
    *INSTALLED.write().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Is a Thai spelling dictionary available (installed or bundled)? When none is, Thai text gets
/// no spelling marks.
pub fn has_dictionary() -> bool {
    dictionary().is_some()
}

/// The `<name>.aff`/`<name>.dic` pairs for [`NAMES`] in `dirs`, in order.
fn pairs_in<'a>(dirs: &'a [PathBuf]) -> impl Iterator<Item = (PathBuf, PathBuf)> + 'a {
    dirs.iter().filter_map(|dir| crate::hunspell::find_in(dir, NAMES))
}

/// Every installed pair in precedence order: user folders first, then system folders. Loading
/// walks this list, so an unreadable user pair falls through to the system rather than to the
/// bundled list.
fn installed_candidates() -> Vec<(PathBuf, PathBuf)> {
    let lo = crate::hunspell::lo_ext_dirs(NAMES.first().copied().unwrap_or("th"));
    let user = crate::hunspell::user_search_dirs();
    let system = crate::hunspell::system_search_dirs(Some(&lo));
    pairs_in(&user).chain(pairs_in(&system)).collect()
}

/// The first candidate that loads with at least one entry. A malformed, empty or over-cap pair
/// is skipped: the next source in precedence order is tried, and the bundled list after that.
fn load_any(candidates: impl IntoIterator<Item = (PathBuf, PathBuf)>) -> Option<Arc<AffixDict>> {
    candidates.into_iter().find_map(|(aff, dic)| match load_thai(&aff, &dic) {
        Ok(d) if !d.is_empty() => Some(Arc::new(d)),
        Ok(_) => {
            log::warn!("Thai spelling: {} has no usable entries", dic.display());
            None
        }
        Err(e) => {
            log::warn!("Thai spelling: {e}");
            None
        }
    })
}

/// Read and sanitize a Thai Hunspell pair. The `.dic` is decoded per the `.aff`'s `SET` line
/// (TIS-620 for the system dictionary) and cleaned before the affix reader sees it.
pub(crate) fn load_thai(aff: &Path, dic: &Path) -> Result<AffixDict, String> {
    let (aff_text, dic_text) = crate::hunspell::read_decoded(aff, dic)?;
    let dic_text = sanitize_dic(&dic_text);
    let source = format!("Hunspell dictionary {}", dic.display());
    AffixDict::from_hunspell(&aff_text, &dic_text, &[source.as_str()])
}

/// Keep only real single Thai words from a `.dic` text, and turn slash-separated alternatives
/// (`บ/ช`) into one entry each.
///
/// A line with whitespace is a phrase (`กรมเอเซียใต้ ตะวันออกกลาง`) or a stem with morphological
/// fields; the affix reader keeps only its first token, which would make a phrase's first word
/// look like a dictionary entry. Skipping such a line whole is safer than mis-parsing it. On a
/// slash line, only the parts that are Thai words survive, so affix flags (`word/A`) and
/// punctuation entries are dropped too. At most [`MAX_ENTRIES`] entries are kept.
fn sanitize_dic(dic: &str) -> String {
    let mut out = String::new();
    let mut kept = 0usize;
    for (n, line) in dic.lines().enumerate() {
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() {
            continue;
        }
        if n == 0 && line.parse::<usize>().is_ok() {
            continue;
        }
        if line.chars().any(char::is_whitespace) {
            continue;
        }
        for part in line.split('/') {
            if !valid_entry(part) || part.len() > crate::affix::MAX_WORD_BYTES {
                continue;
            }
            out.push_str(part);
            out.push('\n');
            kept = kept.saturating_add(1);
            if kept >= MAX_ENTRIES {
                return out;
            }
        }
    }
    out
}

/// Strip the signs that attach to the word before them: maiyamok (ๆ) and paiyannoi (ฯ).
fn base_word(word: &str) -> &str {
    word.trim_end_matches(['\u{0E46}', '\u{0E2F}'])
}

/// Is `word` spelled correctly in Thai? Digits, single graphemes, non-Thai scripts and every
/// word when no dictionary is available count as correct (no marks).
pub fn is_correct(word: &str) -> bool {
    is_correct_with(dictionary().as_ref(), word)
}

pub(crate) fn is_correct_with(dict: Option<&Active>, word: &str) -> bool {
    let Some(d) = dict else { return true };
    if graphemes(word) <= 1 || !word.chars().any(is_thai_letter) {
        return true;
    }
    known(d, word)
}

/// Is one word in the active dictionary, the user dictionary, or a known compound? A trailing
/// maiyamok/paiyannoi is stripped only when the word as written is not itself an entry, so
/// abbreviations such as `ฯลฯ` stay correct.
fn known(d: &Active, w: &str) -> bool {
    if d.contains(w) || crate::in_user_dictionary(w) || is_compound(d, w) {
        return true;
    }
    let base = base_word(w);
    base != w && !base.is_empty() && (d.contains(base) || crate::in_user_dictionary(base) || is_compound(d, base))
}

/// Can `w` be split into two or more words that are each in the dictionary (a compound written
/// solid)? Bounded by [`MAX_COMPOUND_CODEPOINTS`] and only over grapheme boundaries.
fn is_compound(d: &Active, w: &str) -> bool {
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
pub(crate) fn segment_issue(text: &str, segment: Segment, dict: Option<&Active>) -> Option<Issue> {
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
/// never more than [`MAX_SUGGESTIONS`]). The dictionary's `REP` pairs come first, then the
/// closest entries by edit distance.
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    let Some(d) = dictionary() else { return Vec::new() };
    suggest_with(&d, word, max)
}

pub(crate) fn suggest_with(d: &Active, word: &str, max: usize) -> Vec<String> {
    let word = base_word(word);
    let n = word.chars().count();
    if n == 0 || n > MAX_SUGGEST_WORD_CHARS || !word.chars().any(is_thai_letter) {
        return Vec::new();
    }
    let cap = max.min(MAX_SUGGESTIONS);
    if cap == 0 {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    // The dictionary's own corrections first (`REP`), as Hunspell applies them.
    for (from, to) in d.rep() {
        if from.is_empty() || from == to || !word.contains(from.as_str()) {
            continue;
        }
        let candidate = word.replacen(from.as_str(), to.as_str(), 1);
        if candidate != word && d.suggestible(&candidate) && !out.contains(&candidate) {
            out.push(candidate);
            if out.len() >= cap {
                return out;
            }
        }
    }
    let target: Vec<char> = word.chars().collect();
    let mut scored: Vec<(usize, usize, String)> = Vec::new();
    let candidates: Box<dyn Iterator<Item = &str> + '_> = match d {
        Active::Bundled(d) => Box::new(d.iter()),
        Active::Hunspell(d) => Box::new(d.stems().take(MAX_SUGGEST_CANDIDATES)),
    };
    for candidate in candidates {
        let wn = candidate.chars().count();
        if wn + MAX_SUGGEST_DISTANCE < n || wn > n + MAX_SUGGEST_DISTANCE {
            continue;
        }
        let dist = crate::damerau(&target, candidate);
        if dist <= MAX_SUGGEST_DISTANCE && d.suggestible(candidate) {
            scored.push((dist, wn.abs_diff(n), candidate.to_string()));
        }
    }
    scored.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));
    for (_, _, w) in scored {
        if !out.contains(&w) {
            out.push(w);
            if out.len() >= cap {
                break;
            }
        }
    }
    out
}

/// Tests that touch the shared dictionary folder or installed cache take this lock, so tests
/// running in parallel don't see each other's temporary dictionaries.
#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundled_dict() -> Arc<Dictionary> {
        bundled().expect("the bundled CC0 list decompresses")
    }

    #[test]
    fn bundled_list_loads() {
        let d = bundled_dict();
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
        assert!(suggest_with(&Active::Bundled(Arc::new(Dictionary::default())), "ไทน", 5).is_empty());
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
        let d = Active::Bundled(Arc::new(Dictionary::from_text("ประเทศ\nไทย\nน้ำ\nแข็ง\n").expect("dictionary")));
        assert!(is_correct_with(Some(&d), "ประเทศไทย"));
        assert!(is_correct_with(Some(&d), "น้ำแข็ง"));
        assert!(is_correct_with(Some(&d), "ไทย"));
        assert!(!is_correct_with(Some(&d), "ไทน"));
        assert!(!is_correct_with(Some(&d), "ประเทศไทยไทน"));
    }

    /// Encode Thai (and ASCII) as TIS-620, the way the system `th_TH` dictionary stores it.
    fn tis620(text: &str) -> Vec<u8> {
        text.chars()
            .map(|c| {
                let n = u32::from(c);
                if n < 0x80 {
                    n as u8
                } else if (0x0E01..=0x0E3A).contains(&n) || (0x0E3F..=0x0E5B).contains(&n) {
                    (0xA1 + (n - 0x0E01)) as u8
                } else {
                    b'?'
                }
            })
            .collect()
    }

    fn write_pair(dir: &Path, aff: &str, dic: &str) {
        std::fs::write(dir.join("th_TH.aff"), tis620(aff)).unwrap();
        std::fs::write(dir.join("th_TH.dic"), tis620(dic)).unwrap();
    }

    #[test]
    fn decodes_tis620_and_checks_words() {
        let dir = std::env::temp_dir().join(format!("wordcraft-thai-tis-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        write_pair(&dir, "SET TIS620\nREP 1\nREP กกก ไทย\n", "3\nไทย\nภาษา\nดี\n");
        let aff = dir.join("th_TH.aff");
        let dic = dir.join("th_TH.dic");
        let (aff_text, dic_text) = crate::hunspell::read_decoded(&aff, &dic).unwrap();
        assert!(aff_text.contains("SET TIS620"));
        assert!(dic_text.contains("ไทย") && dic_text.contains("ภาษา"));
        let d = load_thai(&aff, &dic).unwrap();
        assert!(d.check("ไทย") && d.check("ภาษา") && d.check("ดี"));
        assert!(!d.check("ไทน"));
        // The TIS-620 REP pair applies to suggestions even when edit distance is over the limit.
        assert_eq!(suggest_with(&Active::Hunspell(Arc::new(d)), "กกก", 5)[0], "ไทย");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sanitizes_phrases_and_slash_alternatives() {
        let dic = "6\nดี\nดีมาก มาก\nดี/ชั่ว\nword/A\nบ/ช\nก.\n";
        let out = sanitize_dic(dic);
        assert_eq!(out, "ดี\nดี\nชั่ว\nบ\nช\n");
        let d = AffixDict::from_hunspell("SET UTF-8\n", &out, &[]).unwrap();
        for w in ["ดี", "ชั่ว", "บ", "ช"] {
            assert!(d.check(w), "{w}");
        }
        // A phrase is skipped whole, not read as its first word; flags aren't words.
        for w in ["มาก", "ดีมาก", "word", "A"] {
            assert!(!d.check(w), "{w}");
        }
        // Over-long entries are dropped; TIS-620 truncation becomes U+FFFD and is dropped too.
        let over = "ก".repeat(MAX_WORD_CODEPOINTS + 1);
        assert_eq!(sanitize_dic(&format!("{over}\nดี\n")), "ดี\n");
        assert_eq!(sanitize_dic("ดี\u{fffd}\n"), "");
        // Reserved TIS-620 bytes decode to U+FFFD, never panic.
        assert!(crate::hunspell::decode(b"SET TIS620\n", &[0x80, 0xDB, 0xFC]).unwrap().1.contains('\u{fffd}'));
    }

    #[test]
    fn user_folder_precedes_system_and_system_precedes_bundled() {
        let base = std::env::temp_dir().join(format!("wordcraft-thai-order-{}", std::process::id()));
        let user = base.join("user");
        let system = base.join("system");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&system).unwrap();
        write_pair(&user, "SET TIS620\n", "1\nผู้ใช้\n");
        write_pair(&system, "SET TIS620\n", "1\nระบบ\n");
        // The same lookup `installed()` runs: user directories first, then system ones.
        let (aff, dic) = pairs_in(&[user.clone(), system.clone()]).next().expect("a pair");
        assert_eq!(dic, user.join("th_TH.dic"));
        let d = load_thai(&aff, &dic).unwrap();
        assert!(d.check("ผู้ใช้") && !d.check("ระบบ"), "the user dictionary wins");
        let (aff, dic) = pairs_in(std::slice::from_ref(&system)).next().expect("the system pair");
        let d = load_thai(&aff, &dic).unwrap();
        assert!(d.check("ระบบ"), "the system dictionary is used when the user has none");
        // With neither pair the bundled list is what remains.
        assert!(pairs_in(std::slice::from_ref(&base)).next().is_none());
        assert!(dictionary().is_some(), "the bundled list still answers");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_malformed_user_dictionary_falls_through_to_the_system_one() {
        let base = std::env::temp_dir().join(format!("wordcraft-thai-fallthrough-{}", std::process::id()));
        let user = base.join("user");
        let system = base.join("system");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&system).unwrap();
        std::fs::write(user.join("th_TH.aff"), "SET TIS620\nPFX\n").unwrap();
        std::fs::write(user.join("th_TH.dic"), "1\nดี\n").unwrap();
        write_pair(&system, "SET TIS620\n", "1\nระบบ\n");
        let d = load_any(pairs_in(&[user, system])).expect("the system pair loads");
        assert!(d.check("ระบบ"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn reload_picks_up_a_newly_installed_or_changed_dictionary() {
        let _guard = test_lock();
        let previous = crate::hunspell::dictionary_dir();
        let dir = std::env::temp_dir().join(format!("wordcraft-thai-reload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let text = "ภาษาไทยเป็นอมพิวเตอร์ที่สวยงาม";
        // No Thai pair yet: the bundled list flags the misspelled segment.
        crate::hunspell::set_dictionary_dir(Some(dir.clone()));
        reload_dictionary();
        let baseline = crate::check_spelling_in(text, ProofLang::Th);
        assert_eq!(baseline.len(), 1, "{baseline:?}");
        let bad = text.get(baseline[0].start..baseline[0].end).unwrap_or("").to_string();
        // Every other segment the text produced is a known word in the installed dictionary, so
        // the only misspelling left is the one this test is about.
        let mut good: Vec<String> = wordcraft_thai::segments(text)
            .into_iter()
            .filter_map(|s| text.get(s.start..s.end))
            .filter(|w| valid_entry(w) && *w != bad)
            .map(str::to_string)
            .collect();
        good.sort_unstable();
        good.dedup();
        let with_bad = {
            let mut w = good.clone();
            w.push(bad.clone());
            w.sort_unstable();
            w
        };
        let dic = |words: &[String]| format!("{}\n{}\n", words.len(), words.join("\n"));
        // A newly installed dictionary that knows the word is picked up by a reload.
        write_pair(&dir, "SET TIS620\n", &dic(&with_bad));
        reload_dictionary();
        assert!(crate::check_spelling_in(text, ProofLang::Th).is_empty());
        // A changed dictionary is picked up too.
        write_pair(&dir, "SET TIS620\n", &dic(&good));
        reload_dictionary();
        assert_eq!(crate::check_spelling_in(text, ProofLang::Th).len(), 1);
        // Restore the shared setting for the other tests.
        crate::hunspell::set_dictionary_dir(previous);
        reload_dictionary();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_truncated_and_hostile_inputs_never_panic() {
        let dir = std::env::temp_dir().join(format!("wordcraft-thai-hostile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Missing files are an error, not a panic.
        assert!(load_thai(&dir.join("missing.aff"), &dir.join("missing.dic")).is_err());
        // A truncated UTF-8 pair is rejected, and the bundled list still answers.
        std::fs::write(dir.join("th_TH.aff"), b"SET UTF-8\n").unwrap();
        std::fs::write(dir.join("th_TH.dic"), [0xE0, 0x0E]).unwrap();
        assert!(crate::hunspell::read_decoded(&dir.join("th_TH.aff"), &dir.join("th_TH.dic")).is_err());
        assert!(dictionary().is_some());
        // An oversized file is refused by size before it is read.
        let big = dir.join("big.aff");
        let f = std::fs::File::create(&big).unwrap();
        f.set_len(64 << 20 | 1).unwrap();
        assert!(crate::hunspell::read_decoded(&big, &dir.join("th_TH.dic")).is_err());
        // A malformed .aff is an error from the affix reader.
        std::fs::write(dir.join("th_TH.aff"), "SET TIS620\nPFX\n").unwrap();
        std::fs::write(dir.join("th_TH.dic"), "1\nดี\n").unwrap();
        assert!(load_thai(&dir.join("th_TH.aff"), &dir.join("th_TH.dic")).is_err());
        // An entry count over the cap is truncated, and a truncated TIS-620 word is dropped.
        let many: String = "ดี\n".repeat(MAX_ENTRIES.saturating_add(10));
        assert!(sanitize_dic(&many).lines().count() <= MAX_ENTRIES);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
