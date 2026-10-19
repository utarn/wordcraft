//! Complex scripts that reorder and stack (#604): Myanmar, Khmer, Devanagari. Cluster and line
//! breaking tests don't depend on installed fonts (a missing font draws .notdef boxes, but the
//! clusters and breaks are the same); shaping tests use a system font when one is installed.

use super::*;
use wordcraft_doc::{Pos, StoryRef};

fn lay(doc: &Document) -> DocLayout {
    let mut c = LayoutCache::new();
    layout(doc, &mut c, &LayoutOptions::default())
}

/// Body paragraph `i`'s layout.
fn para_layout(l: &DocLayout, i: u32) -> &ParaLayout {
    for p in &l.pages {
        for it in &p.items {
            if let Placed::Lines { story: StoryRef::Body, path, para, .. } = it
                && path.0 == vec![i]
            {
                return para;
            }
        }
    }
    panic!("paragraph {i} not laid out");
}

/// "မြန်မာစာ": the syllables မြ | န် | မာ | စာ.
const MYANMAR: &str = "\u{1019}\u{103C}\u{1014}\u{103A}\u{1019}\u{102C}\u{1005}\u{102C}";

#[test]
fn myanmar_syllables_are_single_clusters() {
    let l = lay(&Document::from_text(MYANMAR));
    let pl = para_layout(&l, 0);
    let starts: Vec<usize> = pl.clusters.iter().map(|c| c.start).collect();
    // Grapheme clusters would also start at the aa signs (bytes 15 and 21), splitting မာ and စာ.
    assert_eq!(starts, vec![0, 6, 12, 18]);
    // The caret can't land inside a syllable: every caret offset is a cluster start or the end.
    let offs = pl.line_offsets(0);
    assert!(!offs.contains(&15) && !offs.contains(&21), "{offs:?}");
    // Stacked consonants (virama) and Khmer subscripts (coeng) too.
    let l = lay(&Document::from_text("\u{1019}\u{1039}\u{1018}\u{102C} \u{1781}\u{17D2}\u{1798}\u{17C2}\u{179A}"));
    let pl = para_layout(&l, 0);
    let starts: Vec<usize> = pl.clusters.iter().map(|c| c.start).collect();
    assert_eq!(starts, vec![0, 12, 13, 25]);
}

#[test]
fn myanmar_without_spaces_wraps_between_syllables() {
    let text = MYANMAR.repeat(80);
    let l = lay(&Document::from_text(&text));
    let pl = para_layout(&l, 0);
    assert!(pl.lines.len() > 1, "a long Myanmar paragraph wraps");
    let breaks = wordcraft_doc::cluster::myanmar_breaks(&text);
    // A line may end before each syllable: before မာ and စာ, never before န် (a final).
    for c in &pl.clusters {
        assert_eq!(c.break_after, breaks.contains(&c.end), "break after {}", c.end);
    }
    assert!(pl.clusters.iter().any(|c| c.break_after && c.end == 12) && !pl.clusters.iter().any(|c| c.break_after && c.end == 6));
    for line in pl.lines.iter().skip(1) {
        let start = pl.clusters[line.c0].start;
        assert!(breaks.contains(&start), "line starts at syllable {start}");
    }
    // Lines are full: a syllable break, not an emergency break after one cluster.
    assert!(pl.lines[0].c1 - pl.lines[0].c0 > 10);
}

/// A face with Myanmar letters (an installed system font), or `None` to skip.
fn myanmar_face() -> Option<wordcraft_fonts::FaceRef> {
    wordcraft_fonts::FontDb::global().fallback_for('\u{1000}', 0).map(|f| wordcraft_fonts::FaceRef::of(&f)).filter(|f| f.has_layout_tables())
}

#[test]
fn myanmar_prevowel_is_drawn_before_its_consonant() {
    let Some(face) = myanmar_face() else {
        eprintln!("skipped: no Myanmar font with OpenType tables installed");
        return;
    };
    // ကေ: in memory consonant first, on screen the vowel sign ေ first.
    let ka = face.glyph_for('\u{1000}');
    let g = wordcraft_fonts::shape(&face, "\u{1000}\u{1031}", &[], |c| c);
    assert_eq!(g.len(), 2, "{g:?}");
    assert_eq!(g[1].gid, ka, "the consonant is drawn second: {g:?}");
    assert!(g.iter().all(|x| x.cluster == 0), "one cluster: {g:?}");
    // Itemised by script: the same syllable after Latin text in the same face shapes the same
    // (shaped as one Latin run, the vowel sign would stay after the consonant).
    if face.covers('H') {
        let mixed = wordcraft_fonts::shape(&face, "Hi \u{1000}\u{1031}", &[], |c| c);
        let tail: Vec<u32> = mixed.iter().filter(|x| x.cluster >= 3).map(|x| x.gid).collect();
        assert_eq!(tail, g.iter().map(|x| x.gid).collect::<Vec<_>>());
    }
}

#[test]
fn myanmar_in_a_latin_font_falls_back_to_a_regular_myanmar_face() {
    if myanmar_face().is_none() {
        eprintln!("skipped: no Myanmar font with OpenType tables installed");
        return;
    }
    let l = lay(&Document::from_text(MYANMAR));
    let pl = para_layout(&l, 0);
    let faces: Vec<_> = pl.clusters.iter().map(|c| pl.styles[c.style as usize].face).collect();
    assert!(faces.windows(2).all(|w| w[0].id() == w[1].id()), "one face for the word");
    let f = faces[0];
    assert!(f.has_layout_tables(), "{} can reorder Myanmar", f.family);
    assert!(!f.italic && (f.weight - 400.0).abs() <= 100.0, "a regular weight, not {} {}", f.family, f.style);
    assert!(pl.glyphs.iter().all(|g| g.gid != 0), "no missing glyphs");
    // The caret after the first syllable sits after its glyphs.
    let x6 = l.caret(&Pos::body(0, 6)).map(|c| c.x).unwrap_or(0.0);
    let x0 = l.caret(&Pos::body(0, 0)).map(|c| c.x).unwrap_or(0.0);
    assert!(x6 > x0);
}

/// The issue's run (#1549): Thai letters and digits, Latin letters, digits and the baht sign.
const THAI_RUN: &str = "[D] น้ำ ค่ำ ปั้น ฝั่ง ปู่ ๑๒๓๔๕๖๗๘๙๐ ฿1,250.50";

/// Is a font with Thai letters installed? `false` skips the test.
fn thai_font_installed() -> bool {
    wordcraft_fonts::FontDb::global().fallback_for('\u{0E19}', 0).is_some()
}

/// Body paragraph 0 of `text`, its run set in `font` at 16 pt (`None` keeps the document's
/// default font), and the face of every cluster: (cluster start, face).
fn cluster_faces(text: &str, font: Option<&str>) -> Vec<(usize, wordcraft_fonts::FaceRef)> {
    let mut d = Document::from_text(text);
    let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.runs[0].props.font = font.map(Into::into);
    p.runs[0].props.size = Some(16.0);
    let l = lay(&d);
    let pl = para_layout(&l, 0);
    pl.clusters.iter().map(|c| (c.start, pl.styles[c.style as usize].face)).collect()
}

#[test]
fn a_missing_thai_font_draws_the_whole_run_in_one_thai_face() {
    if !thai_font_installed() {
        eprintln!("skipped: no Thai font installed");
        return;
    }
    // Word substitutes one font (Cordia New) for a missing Thai font, so the Latin letters, the
    // digits and the baht sign keep the Thai letters' face and size. A Thai name the lists don't
    // spell out ("TH Niramit AS") is matched by its "TH " prefix and substituted the same way.
    let db = wordcraft_fonts::FontDb::global();
    for font in ["TH Sarabun New", "Angsana New", "TH Niramit AS"] {
        if db.has_family(font) {
            continue;
        }
        let faces = cluster_faces(THAI_RUN, Some(font));
        let first = faces[0].1;
        let names: Vec<&str> = faces.iter().map(|(_, f)| f.family.as_str()).collect();
        assert!(faces.iter().all(|(_, f)| f.id() == first.id()), "{font}: one face for the run, not {names:?}");
        assert!(first.covers('\u{0E19}') && first.covers('D') && first.covers('1') && first.covers('\u{0E3F}'), "{font} → {}", first.family);
    }
}

#[test]
fn a_thai_substitute_without_latin_hands_the_run_to_one_face_covering_it() {
    if !thai_font_installed() {
        eprintln!("skipped: no Thai font installed");
        return;
    }
    // When the substitute lacks the run's Thai letters (no listed Thai font is installed, so the
    // Latin fallback family stands in) or its Latin ones, the run moves to one face covering all.
    let latin = wordcraft_fonts::word::resolve("Source Sans 3", false, false).face;
    let face = crate::para::thai_run_face("TH Sarabun New", true, latin, THAI_RUN, false, false).expect("one face covers the run");
    assert_ne!(face.id(), latin.id());
    for c in THAI_RUN.chars().filter(|c| !c.is_whitespace()) {
        assert!(face.covers(c), "{} covers {c}", face.family);
    }
    // Only for a Thai family that was substituted.
    assert!(crate::para::thai_run_face("Calibri", true, latin, THAI_RUN, false, false).is_none());
    assert!(crate::para::thai_run_face("TH Sarabun New", false, latin, THAI_RUN, false, false).is_none());
    // A substitute that already covers the run keeps it.
    assert!(crate::para::thai_run_face("TH Sarabun New", true, face, THAI_RUN, false, false).is_none());
    // Text that is neither Thai nor Latin doesn't make the run move.
    assert!(crate::para::thai_run_face("TH Sarabun New", true, latin, "abc \u{2603}", false, false).is_none());
}

#[test]
fn latin_text_in_a_missing_latin_font_stays_in_its_substitute_beside_thai() {
    if !thai_font_installed() {
        eprintln!("skipped: no Thai font installed");
        return;
    }
    // Word keeps Latin text in the Latin font's substitute; only the Thai (or Myanmar) letters
    // fall back. A missing Latin family (and the default body font, Aptos, wherever it is
    // missing) must not move "Hello" into a Thai face.
    let myanmar = myanmar_face().is_some();
    for font in [Some("No Such Latin Font 1549"), None] {
        let requested = font.unwrap_or(wordcraft_doc::styles::BODY_FONT);
        let latin = wordcraft_fonts::word::resolve(requested, false, false).face;
        for text in ["Hello \u{0E2A}\u{0E27}\u{0E31}\u{0E2A}\u{0E14}\u{0E35}", "Hello \u{1019}\u{103C}\u{1014}\u{103A}\u{1019}\u{102C}"] {
            if text.contains('\u{1019}') && !myanmar {
                continue;
            }
            let faces = cluster_faces(text, font);
            let hello: Vec<_> = faces.iter().filter(|(at, _)| *at < 5).collect();
            assert_eq!(hello.len(), 5, "{faces:?}");
            assert!(
                hello.iter().all(|(_, f)| f.id() == latin.id()),
                "{requested}: Hello in {}, not {:?}",
                latin.family,
                hello.iter().map(|(_, f)| f.family.as_str()).collect::<Vec<_>>()
            );
            assert!(faces.iter().any(|(at, f)| *at >= 6 && f.id() != latin.id()), "the other script falls back");
        }
    }
}

#[test]
fn thai_in_an_installed_latin_font_keeps_the_latin_face_for_latin() {
    if !thai_font_installed() {
        eprintln!("skipped: no Thai font installed");
        return;
    }
    // The family exists, so nothing is substituted: Latin stays in it and only the Thai letters
    // fall back, character by character, as before.
    let faces = cluster_faces(THAI_RUN, Some("Source Sans 3"));
    let latin = wordcraft_fonts::word::resolve("Source Sans 3", false, false).face;
    assert!(faces.iter().any(|(_, f)| f.id() == latin.id()), "Latin clusters stay in Source Sans 3");
    assert!(faces.iter().any(|(_, f)| f.id() != latin.id() && f.covers('\u{0E19}')), "Thai clusters fall back to a Thai face");
    assert_eq!(faces[0].1.id(), latin.id(), "[D] opens the run in Source Sans 3");
}

#[test]
fn a_missing_latin_font_with_a_thai_complex_script_font_keeps_both() {
    if !thai_font_installed() {
        eprintln!("skipped: no Thai font installed");
        return;
    }
    // A run with its own complex-script font (`w:cs`) draws the Thai letters in that font's
    // substitute and the Latin letters in the Latin font's substitute, as Word does.
    let mut d = Document::from_text(THAI_RUN);
    let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.runs[0].props.font = Some("No Such Latin Font 1549".into());
    p.runs[0].props.font_cs = Some("TH Sarabun New".into());
    let l = lay(&d);
    let pl = para_layout(&l, 0);
    let faces: Vec<_> = pl.clusters.iter().map(|c| pl.styles[c.style as usize].face).collect();
    let latin = wordcraft_fonts::word::resolve("No Such Latin Font 1549", false, false).face;
    let thai = wordcraft_fonts::word::resolve("TH Sarabun New", false, false).face;
    assert_eq!(faces[0].id(), latin.id(), "[D] in the Latin substitute");
    assert!(faces.iter().any(|f| f.id() == thai.id()), "Thai letters in the Thai substitute {}", thai.family);
}

#[test]
fn a_missing_roman_thai_font_is_not_drawn_in_cambria() {
    // Word draws a missing font the font table classes as roman in Cambria, but a missing Thai
    // font keeps its Thai substitute: the same face whether or not the table calls it roman.
    let db = wordcraft_fonts::FontDb::global();
    if db.has_family("TH Sarabun New") {
        eprintln!("skipped: TH Sarabun New is installed");
        return;
    }
    let face = |roman: bool| {
        let mut d = Document::from_text("\u{0E19}\u{0E49}\u{0E33} Thai");
        if roman {
            d.settings.roman_fonts = vec!["TH Sarabun New".into()];
        }
        let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
        p.runs[0].props.font = Some("TH Sarabun New".into());
        let l = lay(&d);
        let pl = para_layout(&l, 0);
        pl.styles[pl.clusters.last().expect("a cluster").style as usize].face
    };
    let cambria = wordcraft_fonts::word::resolve("Cambria", false, false).face;
    assert_ne!(face(true).id(), cambria.id());
    assert_eq!(face(true).id(), face(false).id());
}

/// `w14:ligatures` (Word writes `historicalDiscretional` on whole Khmer documents) turns `liga`
/// and `clig` off, which breaks Khmer OS syllables: Khmer text is shaped the same either way.
#[test]
fn khmer_ignores_the_ligature_setting() {
    use wordcraft_doc::props::{CharProps, Ligatures};
    let text = "\u{179F}\u{17CA}\u{17BE}\u{1794}\u{17A2}\u{1784}\u{17D2}\u{1780}\u{17C1}\u{178F} \u{1795}\u{17D2}\u{178F}\u{1793}\u{17D2}\u{1791}\u{17B6}\u{1791}\u{17C4}\u{179F}";
    let gids = |ligatures: Option<Ligatures>| {
        let chr = CharProps { font: Some("Khmer OS Siemreap".into()), size: Some(12.0), ligatures, ..Default::default() };
        let mut d = Document::new();
        d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(text, chr))];
        let l = lay(&d);
        para_layout(&l, 0).glyphs.iter().map(|g| g.gid).collect::<Vec<_>>()
    };
    let hd = Ligatures { historical: true, discretional: true, ..Default::default() };
    assert_eq!(gids(Some(hd)), gids(None));
    assert_eq!(gids(Ligatures::from_ooxml("none")), gids(None));
}

/// Thai text without spaces: "Thai is a beautiful language, with pleasing tones."
const THAI_SENTENCE: &str = "ภาษาไทยเป็นภาษาที่สวยงามและมีเสียงวรรณยุกต์ที่ไพเราะ";

/// Narrow body paragraph 0 so Thai wraps many times.
fn narrow(d: &mut Document, by: f32) {
    d.format_paragraphs(&Pos::body(0, 0), &Pos::body(0, 0), &|p| p.indent_right = Some(468.0 - by)).unwrap();
}

fn line_ranges(pl: &ParaLayout) -> Vec<(usize, usize)> {
    pl.lines.iter().map(|l| (l.c0, l.c1)).collect()
}

/// The visible text of each line: the paragraph text under the line with hidden and deleted
/// ranges ([`ParaLayout::left_out`]) removed.
fn line_texts(pl: &ParaLayout, text: &str) -> Vec<String> {
    pl.lines
        .iter()
        .map(|line| {
            let mut out = String::new();
            for (i, c) in text.get(line.start..line.stop).unwrap_or("").char_indices() {
                let at = line.start.saturating_add(i);
                if !pl.left_out.iter().any(|r| r.start <= at && at < r.end) {
                    out.push(c);
                }
            }
            out
        })
        .collect()
}

#[test]
fn thai_without_spaces_wraps_at_dictionary_word_boundaries() {
    let text = THAI_SENTENCE.repeat(8);
    let mut d = Document::from_text(&text);
    narrow(&mut d, 200.0);
    let l = lay(&d);
    let pl = para_layout(&l, 0);
    assert!(pl.lines.len() > 4, "a long Thai paragraph wraps: {} lines", pl.lines.len());
    let breaks = wordcraft_thai::breaks(&text);
    for line in pl.lines.iter().skip(1) {
        assert!(breaks.contains(&line.start), "line starts at a dictionary word boundary: {} in {:?}", line.start, text.get(..line.start));
    }
    for line in pl.lines.iter().take(pl.lines.len().saturating_sub(1)) {
        assert!(breaks.contains(&line.stop), "line ends at a dictionary word boundary: {} in {:?}", line.stop, text.get(..line.stop));
    }
    // Pinned: "ภาษา" | "ไทย" | "เป็น" | "ภาษา" | "ที่" …, never per character. Byte offsets.
    assert_eq!(wordcraft_thai::breaks(THAI_SENTENCE), vec![12, 21, 33, 45, 54, 72, 81, 87, 102, 129, 138]);
    assert!(!breaks.contains(&3) && !breaks.contains(&6));
}

#[test]
fn thai_break_guards_hold_in_layout() {
    // Unknown syllables make the dictionary leave boundaries after leading vowels and before
    // marks; the guards must remove them.
    let text = "กกเกกก\u{0E31}กก\u{0E48}กกฯกกๆก".repeat(30);
    let mut d = Document::from_text(&text);
    narrow(&mut d, 200.0);
    let l = lay(&d);
    let pl = para_layout(&l, 0);
    assert!(pl.lines.len() > 1, "{}", pl.lines.len());
    for line in pl.lines.iter().skip(1) {
        let next = text.get(line.start..).and_then(|t| t.chars().next()).unwrap_or(' ');
        let u = u32::from(next);
        assert!(!matches!(u, 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E), "no line starts on a combining mark: {line:?}");
        assert!(!matches!(next, '\u{0E2F}' | '\u{0E46}'), "paiyannoi and maiyamok stay attached: {line:?}");
    }
    for line in pl.lines.iter().take(pl.lines.len().saturating_sub(1)) {
        let prev = text.get(..line.stop).and_then(|t| t.chars().next_back()).unwrap_or(' ');
        assert!(!matches!(u32::from(prev), 0x0E40..=0x0E44), "no line ends on a leading vowel: {line:?}");
    }
}

#[test]
fn thai_mixed_with_latin_digits_and_spaces_keeps_their_breaks() {
    let text = "ไทย ภาษา ABC 123 ภาษาไทย xyz".repeat(30);
    let mut d = Document::from_text(&text);
    narrow(&mut d, 150.0);
    let l = lay(&d);
    let pl = para_layout(&l, 0);
    let breaks = wordcraft_thai::breaks(&text);
    for line in pl.lines.iter().skip(1) {
        let before = text.get(..line.start).unwrap_or("");
        assert!(breaks.contains(&line.start) || before.ends_with(' '), "line starts at a Thai word boundary or after a space: {}", line.start);
    }
    // Latin and digit tokens stay whole: the dictionary never splits them.
    for token in ["ABC", "123", "xyz"] {
        let mut from = 0;
        while let Some(k) = text.get(from..).and_then(|t| t.find(token)) {
            let start = from + k;
            let end = start + token.len();
            for line in &pl.lines {
                assert!(!(line.start > start && line.start < end), "{token} split at {}", line.start);
            }
            from = end;
        }
    }
}

#[test]
fn hidden_thai_does_not_shift_break_offsets() {
    let visible = THAI_SENTENCE.repeat(6);
    let mut plain = Document::from_text(&visible);
    narrow(&mut plain, 200.0);
    let plain_l = lay(&plain);
    let plain_pl = para_layout(&plain_l, 0);

    let mut d = Document::from_text("");
    let hidden = wordcraft_doc::CharProps { hidden: Some(true), ..Default::default() };
    for c in visible.chars() {
        let n = d.para(StoryRef::Body, &Path::top(0)).unwrap().len();
        d.insert_text(&Pos::body(0, n), &c.to_string(), &wordcraft_doc::CharProps::default()).unwrap();
        d.insert_text(&Pos::body(0, n + c.len_utf8()), "ก", &hidden).unwrap();
    }
    narrow(&mut d, 200.0);
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions::default());
    let pl = para_layout(&l, 0);
    assert_eq!(line_texts(pl, &d.para(StoryRef::Body, &Path::top(0)).unwrap().text), line_texts(plain_pl, &visible));
}

#[test]
fn tracked_deleted_thai_does_not_shift_break_offsets() {
    let visible = THAI_SENTENCE.repeat(6);
    let mut plain = Document::from_text(&visible);
    narrow(&mut plain, 200.0);
    let plain_l = lay(&plain);
    let plain_pl = para_layout(&plain_l, 0);

    let mut d = Document::from_text("");
    let mut ranges = Vec::new();
    for c in visible.chars() {
        let n = d.para(StoryRef::Body, &Path::top(0)).unwrap().len();
        d.insert_text(&Pos::body(0, n), &c.to_string(), &wordcraft_doc::CharProps::default()).unwrap();
        let at = n + c.len_utf8();
        d.insert_text(&Pos::body(0, at), "ก", &wordcraft_doc::CharProps::default()).unwrap();
        ranges.push((at, at + "ก".len()));
    }
    for (a, b) in ranges {
        d.format_range(&Pos::body(0, a), &Pos::body(0, b), &|c| c.del = Some(0)).unwrap();
    }
    narrow(&mut d, 200.0);
    let l = layout(&d, &mut LayoutCache::new(), &LayoutOptions { hide_deleted: true, ..Default::default() });
    let pl = para_layout(&l, 0);
    assert_eq!(line_texts(pl, &d.para(StoryRef::Body, &Path::top(0)).unwrap().text), line_texts(plain_pl, &visible));
}

#[test]
fn word_wrap_off_leaves_thai_on_dictionary_breaks_and_latin_breaking_anywhere() {
    // Thai is never broken per character, even with "Allow Latin text to wrap in the middle of a
    // word" off (#14): the dictionary's breaks are the only ones.
    let text = THAI_SENTENCE.repeat(6);
    let mut d = Document::from_text(&text);
    let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.props.word_wrap = Some(false);
    narrow(&mut d, 200.0);
    let l = lay(&d);
    let pl = para_layout(&l, 0);
    assert!(pl.lines.len() > 1);
    let breaks = wordcraft_thai::breaks(&text);
    let flagged: Vec<usize> = pl.clusters.iter().filter(|c| c.break_after).map(|c| c.end).collect();
    assert_eq!(flagged, breaks, "Thai breaks only at dictionary words");
    for line in pl.lines.iter().skip(1) {
        assert!(breaks.contains(&line.start), "line starts at a word boundary: {}", line.start);
    }

    // Latin still breaks anywhere, as before.
    let latin = "Anextraordinarilylongwordwithoutspaceswrapsmidword";
    let mut d = Document::from_text(latin);
    let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.props.word_wrap = Some(false);
    narrow(&mut d, 200.0);
    let l = lay(&d);
    let pl = para_layout(&l, 0);
    assert!(pl.lines.len() > 1);
    assert!(pl.lines.iter().skip(1).any(|line| !latin.get(..line.start).unwrap_or("").ends_with(' ')), "{:?}", line_ranges(pl));
}

#[test]
fn hostile_million_char_thai_paragraph_lays_out_bounded() {
    let text = THAI_SENTENCE.repeat(20_000);
    assert!(text.chars().count() >= 1_000_000);
    let mut d = Document::from_text(&text);
    // One tall page, so the test measures line breaking, not thousands of page objects.
    d.last_section.page_h = 20_000.0;
    narrow(&mut d, 0.0);
    let l = lay(&d);
    let pl = para_layout(&l, 0);
    assert!(pl.lines.len() > 100, "{}", pl.lines.len());
    for line in pl.lines.iter().skip(1) {
        assert!(line.start > 0 && line.start <= text.len());
    }
    let _ = wordcraft_thai::breaks(&text);
}

// Thai Distributed (#16): Word's line distribution for Thai. These tests sit at the layout
// output seam: they read line ranges, line ends and per-cluster x positions.

use wordcraft_doc::props::Align;

/// Body paragraph 0 with `align` and a content width of `width` points. The text is kept in one
/// paragraph, so an embedded `LINE_BREAK` stays a soft break.
fn aligned(text: &str, align: Align, width: f32) -> Document {
    let mut d = Document::new();
    let mut p = wordcraft_doc::Paragraph::with_text(text, wordcraft_doc::CharProps::default());
    p.props.align = Some(align);
    p.props.indent_right = Some(468.0 - width);
    d.body = vec![wordcraft_doc::para_block(p)];
    d
}

fn layout_of(d: &Document) -> ParaLayout {
    let l = lay(d);
    para_layout(&l, 0).clone()
}

/// The last cluster of the line's content: trailing spaces, markers and breaks hang past it.
fn content_last(pl: &ParaLayout, line: &crate::para::Line) -> usize {
    let mut last = line.c1;
    while last > line.c0 {
        let Some(c) = pl.clusters.get(last - 1) else { break };
        if matches!(
            c.kind,
            crate::para::ClKind::Space
                | crate::para::ClKind::Marker
                | crate::para::ClKind::Tab
                | crate::para::ClKind::LineBreak
                | crate::para::ClKind::PageBreak
                | crate::para::ClKind::ColumnBreak
        ) {
            last -= 1;
        } else {
            break;
        }
    }
    last
}

/// The gap after each content cluster but the last (there is no gap after a line's last unit), in
/// logical order, for a plain left-to-right line.
fn gaps(pl: &ParaLayout, line: &crate::para::Line) -> Vec<f32> {
    let last = content_last(pl, line);
    (line.c0..last.saturating_sub(1))
        .filter_map(|k| {
            let c = pl.clusters.get(k)?;
            let a = line.xs.get(k - line.c0)?;
            let b = line.xs.get(k - line.c0 + 1)?;
            Some(b - a - c.adv)
        })
        .collect()
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.02
}

#[test]
fn thai_distributed_non_last_lines_are_filled_equally() {
    let text = THAI_SENTENCE.repeat(3);
    let d = aligned(&text, Align::ThaiDistribute, 180.0);
    let pl = layout_of(&d);
    assert!(pl.lines.len() >= 2, "{} lines", pl.lines.len());
    for line in pl.lines.iter().take(pl.lines.len().saturating_sub(1)) {
        let g = gaps(&pl, line);
        assert!(!g.is_empty());
        let per = g[0];
        assert!(per > 0.0);
        for x in &g {
            assert!(near(*x, per), "equal gaps on a Thai line: {g:?}");
        }
        assert!(near(line.xs.last().copied().unwrap_or(0.0), line.right), "the line reaches its right edge");
    }
    // The paragraph's last line is laid out exactly as Align Left.
    let left = layout_of(&aligned(&text, Align::Left, 180.0));
    assert_eq!(pl.lines.len(), left.lines.len());
    let (a, b) = (pl.lines.last().unwrap(), left.lines.last().unwrap());
    assert_eq!((a.c0, a.c1), (b.c0, b.c1));
    assert_eq!(a.xs.len(), b.xs.len());
    for (x, y) in a.xs.iter().zip(&b.xs) {
        assert!(near(*x, *y), "last line positions equal Align Left: {x} vs {y}");
    }
}

#[test]
fn thai_distributed_soft_break_line_distributes_but_the_empty_last_line_does_not() {
    let text = format!("ภาษาไทย{}", wordcraft_doc::para::LINE_BREAK);
    let d = aligned(&text, Align::ThaiDistribute, 300.0);
    let pl = layout_of(&d);
    assert_eq!(pl.lines.len(), 2, "{:?}", line_ranges(&pl));
    let first = &pl.lines[0];
    assert_eq!(first.end, LineEnd::LineBreak);
    assert!(near(first.xs.last().copied().unwrap_or(0.0), first.right), "the soft-break line reaches the right edge");
    assert!(gaps(&pl, first).iter().all(|g| *g > 0.0));
    let last = &pl.lines[1];
    assert_eq!((last.c0, last.c1), (pl.clusters.len(), pl.clusters.len()));
    assert_eq!(last.xs.len(), 1);
    assert!(near(last.xs[0], last.left), "the empty final line stays at the start edge");
}

#[test]
fn thai_distributed_units_are_clusters_with_sara_am_separate() {
    // เกา: three clusters, so a gap before ก and before า.
    let d = aligned(&format!("เกา{}", wordcraft_doc::para::LINE_BREAK), Align::ThaiDistribute, 200.0);
    let pl = layout_of(&d);
    let line = &pl.lines[0];
    assert_eq!(content_last(&pl, line) - line.c0, 3, "เกา is three layout clusters");
    let g = gaps(&pl, line);
    assert_eq!(g.len(), 2, "a gap before ก and before า");
    assert!(near(g[0], g[1]) && g[0] > 0.0, "{g:?}");

    // สั้น: only สั้ | น, never a gap inside the first cluster.
    let d = aligned(&format!("สั้น{}", wordcraft_doc::para::LINE_BREAK), Align::ThaiDistribute, 200.0);
    let pl = layout_of(&d);
    let line = &pl.lines[0];
    assert_eq!(content_last(&pl, line) - line.c0, 2, "สั้น separates only as สั้ | น");
    let g = gaps(&pl, line);
    assert_eq!(g.len(), 1);
    assert!(g[0] > 0.0, "{g:?}");

    // นำ: UAX #29 keeps SARA AM in the cluster, but Thai Distributed splits the unit before it.
    let text = format!("นำ{}", wordcraft_doc::para::LINE_BREAK);
    let d = aligned(&text, Align::ThaiDistribute, 200.0);
    let pl = layout_of(&d);
    let line = &pl.lines[0];
    assert_eq!(content_last(&pl, line) - line.c0, 1, "one layout cluster");
    assert_eq!(pl.sara_am.len(), 1, "SARA AM is a unit of its own");
    let cluster = &pl.clusters[line.c0];
    let slack = line.xs[1] - (line.xs[0] + cluster.adv);
    assert!(slack > 0.0, "the gap before SARA AM widens the cluster's run: {slack}");
    // The base glyph stays and the SARA AM glyphs move by the gap.
    let left = layout_of(&aligned(&text, Align::Left, 200.0));
    let (a, b) = (&pl.glyphs, &left.glyphs);
    assert_eq!(a.len(), b.len());
    let moved = a.iter().zip(b).map(|(x, y)| x.dx - y.dx).fold(0.0f32, f32::max);
    let still = a.iter().zip(b).map(|(x, y)| x.dx - y.dx).fold(f32::INFINITY, f32::min);
    assert!(near(moved, slack), "SARA AM's glyphs move by the gap: {moved} vs {slack}");
    assert!(near(still, 0.0), "the consonant before it stays: {still}");
}

#[test]
fn thai_distributed_latin_digit_and_punctuation_runs_are_one_unit() {
    let text = format!("ABC 123 AB-C{}", wordcraft_doc::para::LINE_BREAK);
    let d = aligned(&text, Align::ThaiDistribute, 300.0);
    let pl = layout_of(&d);
    let line = &pl.lines[0];
    let g = gaps(&pl, line);
    // Gaps appear only after the two spaces: between ABC and 123, and between 123 and AB-C.
    let space_gaps: Vec<f32> = (line.c0..line.c1)
        .filter(|k| pl.clusters.get(*k).is_some_and(|c| c.kind == crate::para::ClKind::Space))
        .filter_map(|k| g.get(k - line.c0).copied())
        .collect();
    assert_eq!(space_gaps.len(), 2);
    assert!(space_gaps.iter().all(|x| *x > 0.0) && near(space_gaps[0], space_gaps[1]), "{space_gaps:?}");
    let last = content_last(&pl, line);
    for k in line.c0..last.saturating_sub(1) {
        if pl.clusters.get(k).is_some_and(|c| c.kind != crate::para::ClKind::Space) {
            assert!(near(g[k - line.c0], 0.0), "no gap inside Latin, digits or punctuation: {} at {k}", g[k - line.c0]);
        }
    }
}

#[test]
fn thai_distributed_spaces_absorb_the_gap_and_double_spaces_stay_double() {
    let text = format!("ภาษา ไทย{}", wordcraft_doc::para::LINE_BREAK);
    let d = aligned(&text, Align::ThaiDistribute, 300.0);
    let pl = layout_of(&d);
    let line = &pl.lines[0];
    let g = gaps(&pl, line);
    let per = g[0];
    assert!(per > 0.0);
    let space = (line.c0..line.c1).find(|k| pl.clusters.get(*k).is_some_and(|c| c.kind == crate::para::ClKind::Space)).unwrap();
    assert!(near(g[space - line.c0], per), "the space absorbs the gap after it");
    for (i, x) in g.iter().enumerate() {
        let k = line.c0 + i;
        let expect = if k + 1 == space { 0.0 } else { per };
        assert!(near(*x, expect), "one equal gap between every pair of Thai clusters: {g:?}");
    }

    let text = format!("ภาษา  ไทย{}", wordcraft_doc::para::LINE_BREAK);
    let d = aligned(&text, Align::ThaiDistribute, 300.0);
    let pl = layout_of(&d);
    let line = &pl.lines[0];
    let g = gaps(&pl, line);
    let spaces: Vec<usize> = (line.c0..line.c1).filter(|k| pl.clusters.get(*k).is_some_and(|c| c.kind == crate::para::ClKind::Space)).collect();
    assert_eq!(spaces.len(), 2);
    let first = line.xs[spaces[0] - line.c0];
    let second = line.xs[spaces[1] - line.c0];
    let next = line.xs[spaces[1] - line.c0 + 1];
    let space_adv = pl.clusters[spaces[0]].adv;
    assert!(near(second - first, space_adv), "the two spaces keep their advances");
    assert!(near(g[spaces[0] - line.c0], 0.0), "no gap between two spaces");
    let per = g[spaces[1] - line.c0];
    assert!(per > 0.0 && near(next - second - space_adv, per), "one gap after both spaces");
    assert!(near(line.xs.last().copied().unwrap_or(0.0), line.right));
}

#[test]
fn thai_distributed_starts_after_the_last_tab() {
    let text = format!("AB\tCD EF{}", wordcraft_doc::para::LINE_BREAK);
    let d = aligned(&text, Align::ThaiDistribute, 300.0);
    let left = layout_of(&aligned(&text, Align::Left, 300.0));
    let pl = layout_of(&d);
    let (line, plain) = (&pl.lines[0], &left.lines[0]);
    let tab = (line.c0..line.c1).find(|k| pl.clusters.get(*k).is_some_and(|c| c.kind == crate::para::ClKind::Tab)).expect("the line has a tab");
    for k in line.c0..=tab {
        assert!(near(line.xs[k - line.c0], plain.xs[k - plain.c0]), "tab stops and earlier content do not move at {k}");
    }
    assert!(near(line.xs.last().copied().unwrap_or(0.0), line.right), "content after the tab fills the right edge");
    assert!(gaps(&pl, line)[tab - line.c0..].iter().any(|g| *g > 0.0));
}

#[test]
fn thai_distributed_keeps_indents_list_labels_and_the_left_edge() {
    let text = format!("ภาษาไทย{}", wordcraft_doc::para::LINE_BREAK);
    let mut d = aligned(&text, Align::ThaiDistribute, 300.0);
    let id = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    d.format_paragraphs(&Pos::body(0, 0), &Pos::body(0, 0), &|p| {
        p.numbering = Some(wordcraft_doc::props::NumRef { num: id, level: 0 });
        p.indent_left = Some(30.0);
        p.indent_first = Some(-12.0);
    })
    .unwrap();
    let mut plain = aligned(&text, Align::Left, 300.0);
    let id = plain.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    plain
        .format_paragraphs(&Pos::body(0, 0), &Pos::body(0, 0), &|p| {
            p.numbering = Some(wordcraft_doc::props::NumRef { num: id, level: 0 });
            p.indent_left = Some(30.0);
            p.indent_first = Some(-12.0);
        })
        .unwrap();
    let pl = layout_of(&d);
    let left = layout_of(&plain);
    let (line, base) = (&pl.lines[0], &left.lines[0]);
    assert!(near(line.left, base.left), "the left edge stays put");
    assert!(near(line.xs[0], base.xs[0]), "the first cluster stays at the indent");
    let lab = pl.label.as_ref().expect("a list label");
    let base_lab = left.label.as_ref().expect("a list label");
    assert!(near(lab.x, base_lab.x), "the label stays put");
    assert!(near(line.xs.last().copied().unwrap_or(0.0), line.right), "the right edge is filled");
}

#[test]
fn thai_distributed_fills_the_narrowed_span_beside_a_float() {
    let mut d = aligned(&format!("ภาษาไทย{}", wordcraft_doc::para::LINE_BREAK), Align::ThaiDistribute, 400.0);
    let float = wordcraft_doc::para::Float {
        wrap: wordcraft_doc::para::Wrap::Square,
        h_rel: wordcraft_doc::para::Anchor::Column,
        v_rel: wordcraft_doc::para::Anchor::Paragraph,
        x: 252.0,
        y: 0.0,
        dist: 9.0,
        ..Default::default()
    };
    let shape = InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::Rectangle,
        w: 120.0,
        h: 100.0,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float,
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    d.insert_object(&Pos::body(0, 0), shape, &Default::default()).unwrap();
    let l = lay(&d);
    let pl = para_layout(&l, 0);
    let line = pl.lines.iter().find(|line| line.c1 > line.c0).expect("a text line");
    assert!(line.right < 400.0, "the float narrows the line: {}", line.right);
    assert!(near(line.xs.last().copied().unwrap_or(0.0), line.right), "the line fills the narrowed span");
    assert!(gaps(pl, line).iter().any(|g| *g > 0.0));
}

#[test]
fn thai_distributed_one_unit_empty_and_overflowing_lines_do_not_change() {
    // One unit: no gap and never centred.
    let text = format!("ก{}", wordcraft_doc::para::LINE_BREAK);
    let pl = layout_of(&aligned(&text, Align::ThaiDistribute, 200.0));
    let left = layout_of(&aligned(&text, Align::Left, 200.0));
    assert_eq!(pl.lines[0].xs, left.lines[0].xs);
    // Empty paragraph.
    let d = aligned("", Align::ThaiDistribute, 200.0);
    let l = lay(&d);
    let pl = para_layout(&l, 0);
    assert!(pl.lines.iter().flat_map(|line| &line.xs).all(|x| x.is_finite()));
    // An unbreakable token wider than the line: negative slack distributes nothing.
    let text = format!("Anextraordinarilylongwordwithoutspaces{}{}", "x".repeat(80), wordcraft_doc::para::LINE_BREAK);
    let pl = layout_of(&aligned(&text, Align::ThaiDistribute, 120.0));
    let left = layout_of(&aligned(&text, Align::Left, 120.0));
    assert_eq!(pl.lines[0].xs, left.lines[0].xs);
    assert!(pl.lines.iter().flat_map(|line| &line.xs).all(|x| x.is_finite()));
}

#[test]
fn generic_distributed_keeps_its_behaviour() {
    // Generic Distributed distributes the last line too, unlike Thai Distributed.
    let text = "AB CD EF";
    let dist = layout_of(&aligned(text, Align::Distribute, 300.0));
    let left = layout_of(&aligned(text, Align::Left, 300.0));
    let line = &dist.lines[0];
    assert_eq!(line.end, LineEnd::Para);
    assert!(line.xs.last().copied().unwrap_or(0.0) > left.lines[0].xs.last().copied().unwrap_or(0.0) + 1.0);
    assert!(near(line.xs.last().copied().unwrap_or(0.0), line.right));
    // Thai Distributed leaves that last line at the start edge.
    let thai = layout_of(&aligned(text, Align::ThaiDistribute, 300.0));
    assert_eq!(thai.lines[0].xs, left.lines[0].xs);
}

#[test]
fn thai_distributed_gaps_survive_bidi_reordering() {
    // Thai in a left-to-right paragraph with an embedded right-to-left run.
    let text = format!("ภาษา עברית ไทย{}", wordcraft_doc::para::LINE_BREAK);
    let d = aligned(&text, Align::ThaiDistribute, 320.0);
    let pl = layout_of(&d);
    let line = &pl.lines[0];
    assert!(!line.vis.is_empty(), "the line is bidirectional");
    assert!(line.vis.iter().all(|v| v.x.is_finite() && v.w.is_finite() && v.w >= 0.0));
    let right = line.vis.iter().map(|v| v.x + v.w).fold(f32::MIN, f32::max);
    assert!(near(right, line.right), "the gaps still reach the line's end: {right} vs {}", line.right);
    // A right-to-left paragraph with Thai text.
    let mut d = aligned(&format!("ภาษาไทย{}", wordcraft_doc::para::LINE_BREAK), Align::ThaiDistribute, 300.0);
    d.format_paragraphs(&Pos::body(0, 0), &Pos::body(0, 0), &|p| p.bidi = Some(true)).unwrap();
    let pl = layout_of(&d);
    let line = &pl.lines[0];
    assert!(line.rtl);
    assert!(line.vis.iter().all(|v| v.x.is_finite() && v.w.is_finite() && v.w >= 0.0));
    let mut boxes: Vec<(f32, f32)> = line.vis.iter().filter(|v| v.w > 0.0).map(|v| (v.x, v.x + v.w)).collect();
    boxes.sort_by(|a, b| a.0.total_cmp(&b.0));
    for pair in boxes.windows(2) {
        assert!(pair[1].0 >= pair[0].1 - 0.02, "no overlap after reordering: {boxes:?}");
    }
    assert!(near(boxes.first().map_or(0.0, |b| b.0), line.left), "the line still starts at its edge");
    assert!(near(boxes.last().map_or(0.0, |b| b.1), line.right), "and ends at the other");
}