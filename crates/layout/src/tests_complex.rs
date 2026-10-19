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

/// The face of every cluster of body paragraph `i`, as laid out (`cluster_faces` for a new doc).
fn cluster_faces_from(l: &DocLayout, i: u32) -> Vec<(usize, wordcraft_fonts::FaceRef)> {
    let pl = para_layout(l, i);
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
fn a_run_containing_thai_resolves_once_for_the_whole_thai_portion() {
    if !thai_font_installed() {
        eprintln!("skipped: no Thai font installed");
        return;
    }
    // An installed family that lacks Thai keeps the Latin text in the same family; the Thai
    // letters, marks, digits and baht sign move to one face, not one character at a time.
    let faces = cluster_faces(THAI_RUN, Some("Source Sans 3"));
    let latin = wordcraft_fonts::word::resolve("Source Sans 3", false, false).face;
    let is_thai_at = |at: usize| THAI_RUN.get(at..).and_then(|t| t.chars().next()).is_some_and(wordcraft_fonts::thai::is_thai);
    let thai: Vec<&(usize, wordcraft_fonts::FaceRef)> = faces.iter().filter(|(at, _)| is_thai_at(*at)).collect();
    let first = thai.first().map(|(_, f)| *f).expect("Thai clusters");
    assert!(first.covers('\u{0E19}') && first.covers('\u{0E3F}'), "{} draws Thai", first.family);
    assert!(thai.iter().all(|(_, f)| f.id() == first.id()), "one Thai face: {:?}", thai.iter().map(|(_, f)| f.family.as_str()).collect::<Vec<_>>());
    assert!(faces.iter().filter(|(at, _)| !is_thai_at(*at)).all(|(_, f)| f.id() == latin.id()), "Latin letters and digits stay in Source Sans 3");
    assert!(faces.iter().any(|(at, _)| is_thai_at(*at)));
    // A face only for the Thai portion, never for the Latin text beside it.
}

#[test]
fn a_missing_thai_family_keeps_one_line_height_for_thai_only_and_mixed_paragraphs() {
    if !thai_font_installed() {
        eprintln!("skipped: no Thai font installed");
        return;
    }
    // Word settles a missing Thai family on one face for the run, so a paragraph that switches
    // from Thai only to Thai+Latin must not grow: both are drawn in the same substituted face.
    let db = wordcraft_fonts::FontDb::global();
    if db.has_family("TH Sarabun New") {
        eprintln!("skipped: TH Sarabun New is installed");
        return;
    }
    let mut d = Document::from_text("\u{0E44}\u{0E17}\u{0E22}\n\u{0E44}\u{0E17}\u{0E22} ABC 123");
    for i in 0..2 {
        let p = d.para_mut(StoryRef::Body, &Path::top(i)).unwrap();
        p.runs[0].props.font = Some("TH Sarabun New".into());
        p.runs[0].props.size = Some(16.0);
    }
    let l = lay(&d);
    let heights: Vec<f32> = (0..2).map(|i| para_layout(&l, i).lines[0].height).collect();
    assert!((heights[0] - heights[1]).abs() < 0.01, "Thai-only and mixed paragraphs keep one line height: {heights:?}");
    let faces: Vec<_> = (0..2).flat_map(|i| cluster_faces_from(&l, i)).collect();
    assert!(faces.iter().all(|(_, f)| f.id() == faces[0].1.id()), "one substituted face for both paragraphs");
}

#[test]
fn a_theme_thai_typeface_becomes_the_requested_thai_family() {
    if !thai_font_installed() {
        eprintln!("skipped: no Thai font installed");
        return;
    }
    // A `w:cstheme` run arrives with the theme's Thai typeface kept apart from the theme's
    // generic complex-script font (and the theme slot kept for the save). Layout must draw the
    // Thai portion in the Thai typeface, not the generic complex-script or Latin one.
    use wordcraft_fonts::thai::{Role, current_platform, first_installed};
    let db = wordcraft_fonts::FontDb::global();
    let Some(theme_family) = first_installed(Role::Sans, current_platform(), db) else {
        eprintln!("skipped: no Thai face from the ordered policy");
        return;
    };
    let text = "\u{0E44}\u{0E17}\u{0E22} ABC 123";
    let mut d = Document::from_text(text);
    let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.runs[0].props.font = Some("Source Sans 3".into());
    p.runs[0].props.font_cs = Some("No Such Generic CS Font 18".into());
    p.runs[0].props.font_cs_thai = Some(theme_family.into());
    p.runs[0].props.font_cs_theme = Some("minorBidi".into());
    let l = lay(&d);
    let pl = para_layout(&l, 0);
    let face_at = |start: usize| pl.clusters.iter().find(|c| c.start == start).map(|c| pl.styles[c.style as usize].face);
    assert!(face_at(0).is_some_and(|f| f.family == theme_family), "Thai uses the theme's Thai typeface");
    assert!(face_at(10).is_some_and(|f| f.family == "Source Sans 3"), "Latin keeps its own family");
}

#[test]
fn a_thai_capable_latin_family_that_is_missing_keeps_latin_and_draws_thai_in_one_face() {
    if !thai_font_installed() {
        eprintln!("skipped: no Thai font installed");
        return;
    }
    // Tahoma is a Thai-capable Latin design. When it is missing, Word keeps its Latin substitute
    // (Carlito, the metric-compatible Calibri face) and draws the whole Thai portion in one Thai
    // face, without shredding the run per character.
    let db = wordcraft_fonts::FontDb::global();
    if db.has_family("Tahoma") {
        eprintln!("skipped: Tahoma is installed");
        return;
    }
    let faces = cluster_faces(THAI_RUN, Some("Tahoma"));
    let latin = wordcraft_fonts::word::resolve("Tahoma", false, false).face;
    let is_thai_at = |at: usize| THAI_RUN.get(at..).and_then(|t| t.chars().next()).is_some_and(wordcraft_fonts::thai::is_thai);
    let thai_faces: Vec<wordcraft_fonts::FaceRef> = faces.iter().filter(|(at, _)| is_thai_at(*at)).map(|(_, f)| *f).collect();
    let first = *thai_faces.first().expect("Thai clusters");
    assert!(first.covers('\u{0E19}'), "{} draws Thai", first.family);
    assert!(thai_faces.iter().all(|f| f.id() == first.id()), "one Thai face for the whole portion");
    assert!(faces.iter().filter(|(at, _)| !is_thai_at(*at)).all(|(_, f)| f.id() == latin.id()), "[D] and the ASCII digits stay in {}", latin.family);
    // The Thai face is a face from the shared policy, not the Latin substitute.
    assert_ne!(first.id(), latin.id());
}

#[test]
fn thai_marks_and_sara_am_never_start_a_new_face() {
    if !thai_font_installed() {
        eprintln!("skipped: no Thai font installed");
        return;
    }
    // "น้ำ" holds SARA AM after its consonant, and "ปั้น" a mark after a vowel sign. Neither may
    // begin a face of its own when the family lacks Thai.
    let text = "\u{0E19}\u{0E49}\u{0E33}\u{0E1B}\u{0E31}\u{0E49}\u{0E19}";
    let faces = cluster_faces(text, Some("Source Sans 3"));
    let thai: Vec<wordcraft_fonts::FaceRef> = faces.iter().map(|(_, f)| *f).collect();
    assert!(!thai.is_empty());
    assert!(thai.iter().all(|f| f.id() == thai[0].id()), "marks and SARA AM keep the word's face");
    assert!(thai[0].covers('\u{0E19}') && thai[0].covers('\u{0E33}'));
}

#[test]
fn a_cs_run_supplies_the_complex_script_family_and_its_size() {
    if !thai_font_installed() {
        eprintln!("skipped: no Thai font installed");
        return;
    }
    // `w:cs` names the Thai family; it follows the same fallback rules as the Latin slot, and the
    // Thai portion is drawn at the complex-script size (`w:szCs`).
    let mut d = Document::from_text(THAI_RUN);
    let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.runs[0].props.font = Some("No Such Latin Font 1549".into());
    p.runs[0].props.font_cs = Some("TH Sarabun New".into());
    p.runs[0].props.size = Some(11.0);
    p.runs[0].props.size_cs = Some(16.0);
    p.runs[0].props.cs = Some(true);
    let l = lay(&d);
    let pl = para_layout(&l, 0);
    let latin = wordcraft_fonts::word::resolve("No Such Latin Font 1549", false, false).face;
    let thai = wordcraft_fonts::word::resolve("TH Sarabun New", false, false).face;
    let at = |start: usize| pl.clusters.iter().find(|c| c.start == start).map(|c| &pl.styles[c.style as usize]);
    // "[D]" is Latin and keeps the Latin substitute; the Thai letters use the requested cs family.
    let d_style = at(1).expect("[D]");
    assert_eq!(d_style.face.id(), latin.id(), "[D] in the Latin substitute");
    let thai_style = at(4).expect("Thai letters");
    assert_eq!(thai_style.face.id(), thai.id(), "Thai in the requested complex-script family");
    assert!(pl.styles.iter().filter(|s| s.face.id() == thai.id()).all(|s| (s.size - 16.0).abs() < 0.25), "the Thai portion uses w:szCs");
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
