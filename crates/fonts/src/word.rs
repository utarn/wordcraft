//! Word-processor font resolution: map a requested family + bold/italic to an available face,
//! with metric-compatible substitutes for common proprietary document fonts, and Word-style line
//! metrics (Windows ascent/descent, which is what Word uses for single line spacing).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use skrifa::raw::TableProvider;

use crate::{FaceRef, FontDb};

/// A requested font resolved to a face.
#[derive(Clone, Copy, Debug)]
pub struct Resolved {
    pub face: FaceRef,
    /// The face isn't bold but bold was asked for: embolden when drawing.
    pub synth_bold: bool,
    /// The face isn't italic but italic was asked for: slant when drawing.
    pub synth_italic: bool,
    /// The requested family wasn't available.
    pub substituted: bool,
}

/// Substitutes to try, in order, for a family that isn't installed. Open fonts that are
/// metric-compatible come first (Carlito ↔ Calibri, Caladea ↔ Cambria, Liberation ↔ Arial /
/// Times New Roman / Courier New), then common system fonts, then our bundled families.
fn substitutes(family: &str) -> &'static [&'static str] {
    substitutes_by_name(family).0
}

/// Does `family` have substitutes of its own, picked by its name (a metric-compatible clone such
/// as Liberation Serif for Times New Roman, or the same design under another name), rather than
/// the generic lists any unknown font gets?
pub fn has_named_substitutes(family: &str) -> bool {
    substitutes_by_name(charset_base_family(family)).1
}

/// [`substitutes`], and whether they were chosen for this family by name.
fn substitutes_by_name(family: &str) -> (&'static [&'static str], bool) {
    const SANS: &[&str] = &["Carlito", "Helvetica Neue", "Arial", "Liberation Sans", "Noto Sans", "DejaVu Sans", "Source Sans 3"];
    const SERIF: &[&str] = &["Caladea", "Times New Roman", "Liberation Serif", "Georgia", "Noto Serif", "DejaVu Serif", "Source Serif 4"];
    const MONO: &[&str] = &["Courier New", "Liberation Mono", "Menlo", "Consolas", "DejaVu Sans Mono", "JetBrains Mono"];
    const ARIAL: &[&str] = &["Liberation Sans", "Helvetica", "Helvetica Neue", "Arimo", "Source Sans 3"];
    const ARIAL_UNICODE: &[&str] = &["Arial", "Liberation Sans", "Helvetica", "Helvetica Neue", "Arimo", "Source Sans 3"];
    const TIMES: &[&str] = &["Liberation Serif", "Times", "Tinos", "Source Serif 4"];
    // Word's symbol fonts (Symbol, Wingdings…) are proprietary: their characters are drawn as their
    // Unicode equivalents (`wordcraft_doc::symbol_fonts`) from these faces or any face that has them.
    const SYMBOL: &[&str] = &["Apple Symbols", "Segoe UI Symbol", "DejaVu Sans", "Noto Sans Symbols", "Noto Sans Symbols 2", "Source Sans 3"];
    // Persian / Arabic document fonts. Naskh (book) faces such as B Nazanin, B Lotus, B Zar and
    // Traditional Arabic, then the sans faces (B Yekan, B Titr, IRANSans) people use for
    // headings and screens. Open fonts with full Persian coverage first.
    const NASKH: &[&str] = &["Noto Naskh Arabic", "XB Zar", "XB Niloofar", "Amiri", "Vazirmatn", "Noto Sans Arabic", "DejaVu Sans", "Source Serif 4"];
    const ARABIC_SANS: &[&str] = &["Vazirmatn", "Sahel", "Shabnam", "Samim", "Noto Sans Arabic", "Noto Naskh Arabic", "DejaVu Sans", "Source Sans 3"];
    // Hebrew document fonts: platform-specific fonts first, then common open source fallbacks.
    // Windows: Microsoft Office fonts (David, Miriam) are common; Segoe UI is standard.
    // macOS: Helvetica has good Hebrew support; Geeza Pro is included by default.
    // Linux: Noto Sans Hebrew is the best option.
    #[cfg(windows)]
    const HEBREW: &[&str] = &["Noto Sans Hebrew", "David", "Miriam", "Segoe UI", "Arial", "Tahoma", "DejaVu Sans", "Source Sans 3"];
    #[cfg(target_os = "macos")]
    const HEBREW: &[&str] = &["Helvetica", "Noto Sans Hebrew", "Arial", "Tahoma", "Geeza Pro", "DejaVu Sans", "Source Sans 3"];
    #[cfg(not(any(windows, target_os = "macos")))]
    const HEBREW: &[&str] = &["Noto Sans Hebrew", "Segoe UI", "Arial", "Tahoma", "DejaVu Sans", "Source Sans 3"];
    // Thai document fonts (TH Sarabun New, Cordia New, the UPC families…) resolve through the
    // single ordered policy in [`crate::thai`], shared with script fallback (#15). The policy
    // handles both Thai sans and Thai serif roles; Latin families that merely carry Thai
    // (Tahoma, Microsoft Sans Serif, Arial Unicode MS) keep the Latin substitutes below.
    // Khmer document fonts (Khmer OS …, Khmer Mool1…): a missing one is drawn in the system's
    // Khmer font, as Word does, not in whatever Khmer font the document happens to load: the Khmer
    // OS fonts are wider, so lines wrap and boxes overflow where Word's don't.
    #[cfg(windows)]
    const KHMER: &[&str] = &["Leelawadee UI", "Khmer UI", "DaunPenh", "MoolBoran", "Noto Sans Khmer"];
    #[cfg(target_os = "macos")]
    const KHMER: &[&str] = &["Khmer Sangam MN", "Khmer MN", "Noto Sans Khmer"];
    #[cfg(not(any(windows, target_os = "macos")))]
    const KHMER: &[&str] = &["Noto Sans Khmer", "Noto Serif Khmer"];
    // Palatino Linotype / Book Antiqua: macOS ships the same design as "Palatino"; open
    // look-alikes with matching metrics (TeX Gyre Pagella, URW P052 / Palladio) on Linux.
    const PALATINO: &[&str] = &[
        "Palatino Linotype",
        "Palatino",
        "Book Antiqua",
        "TeX Gyre Pagella",
        "P052",
        "URW Palladio L",
        "Palladio",
        "Caladea",
        "Times New Roman",
        "Liberation Serif",
        "Georgia",
        "Noto Serif",
        "DejaVu Serif",
        "Source Serif 4",
    ];
    let f = family.to_ascii_lowercase();
    if let Some(role) = crate::thai::role(&f)
        && role != crate::thai::Role::CapableLatin
    {
        return (crate::thai::candidates(role, crate::thai::current_platform()), true);
    }
    if is_hebrew(&f) {
        return (HEBREW, true);
    }
    if is_khmer(&f) {
        return (KHMER, true);
    }
    if is_persian_naskh(&f) {
        return (NASKH, true);
    }
    if is_persian_sans(&f) {
        return (ARABIC_SANS, true);
    }
    match f.as_str() {
        "palatino linotype" | "book antiqua" | "palatino" => (PALATINO, true),
        "arial" | "helvetica" | "arial nova" => (ARIAL, true),
        "arial unicode ms" => (ARIAL_UNICODE, true),
        "times new roman" | "times" => (TIMES, true),
        "courier new" | "courier" | "consolas" | "lucida console" | "cascadia code" | "cascadia mono" => (MONO, true),
        "symbol" | "wingdings" | "wingdings 2" | "wingdings 3" | "segoe ui symbol" | "webdings" => (SYMBOL, true),
        // Serif faces with no open clone of their own: the generic serif list, as for any font.
        "cambria"
        | "cambria math"
        | "georgia"
        | "garamond"
        | "constantia"
        | "baskerville old face"
        | "century"
        | "century schoolbook"
        | "bookman old style"
        | "sitka text"
        | "sitka"
        | "aptos serif" => (SERIF, false),
        _ if f.contains("mono") || f.contains("code") => (MONO, false),
        _ if f.contains("serif") && !f.contains("sans") => (SERIF, false),
        _ => (SANS, false),
    }
}

/// Ascent and descent (em) of Word's symbol fonts, which aren't open fonts. Their characters are
/// drawn from other faces (`wordcraft_doc::symbol_fonts`), but Word sizes a line by the font
/// itself: a Symbol bullet makes its line taller than the text alone. The values are those Word's
/// PDFs give the fonts (`/Ascent`, `/Descent`).
pub fn symbol_font_metrics(family: &str) -> Option<(f64, f64)> {
    match family.trim().to_ascii_lowercase().as_str() {
        "symbol" | "symbol (pcl6)" => Some((1.005, 0.216)),
        "wingdings" => Some((0.899, 0.205)),
        _ => None,
    }
}

/// The (ascent, descent) in em Word gives a line with a run in a symbol font, when that isn't the
/// resolved face's own: the font isn't installed (`resolved` is a substitute), or the installed
/// face is one em tall, as Apple's Symbol is (hhea 0.701 / -0.299, no gap). Word for Mac spaces
/// such lines like Windows' Symbol: 1.222 em at 11 and 20 pt on synthetic documents (#2681).
pub fn symbol_line(family: &str, resolved: &Resolved) -> Option<(f64, f64)> {
    let m = symbol_font_metrics(family)?;
    (resolved.substituted || one_em(line_metrics(&resolved.face), resolved.face.upem)).then_some(m)
}

/// Is a line's (ascent, descent) one `upem` em tall?
fn one_em((ascent, descent): (f64, f64), upem: f64) -> bool {
    upem > 0.0 && ((ascent + descent) / upem - 1.0).abs() < 0.005
}

/// The font a Windows 3.1-era charset name stands for: `Times New Roman Cyr`, `Arial CE`,
/// `Courier New Baltic`, `Arial (Hebrew)`… are aliases Windows keeps (its FontSubstitutes) for
/// the font itself, and old Russian and Central European documents name them. Other names are
/// returned unchanged.
pub fn charset_base_family(family: &str) -> &str {
    const SUFFIXES: &[&str] = &[" cyr", " ce", " greek", " tur", " baltic", " (arabic)", " (hebrew)", " (vietnamese)"];
    let lower = family.trim_end().to_ascii_lowercase();
    SUFFIXES
        .iter()
        .find(|s| lower.ends_with(*s) && lower.len() > s.len())
        .and_then(|s| family.trim_end().get(..lower.len() - s.len()))
        .map_or(family, str::trim_end)
}

/// Naskh-style Persian/Arabic document fonts (the "B …" family names come from the common
/// Persian font packs; matched by name only).
fn is_persian_naskh(f: &str) -> bool {
    const NAMES: &[&str] = &[
        "b nazanin",
        "b lotus",
        "b zar",
        "b mitra",
        "b badr",
        "b yagut",
        "b roya",
        "b compset",
        "b nazanin bold",
        "b lotus bold",
        "nazanin",
        "lotus",
        "zar",
        "mitra",
        "badr",
        "yagut",
        "roya",
        "ir nazanin",
        "ir lotus",
        "ir zar",
        "ir mitra",
        "ir badr",
        "irnazanin",
        "irlotus",
        "irzar",
        "irmitra",
        "irbadr",
        "traditional arabic",
        "simplified arabic",
        "arabic typesetting",
        "sakkal majalla",
        "adobe arabic",
        "andalus",
        "times new roman (arabic)",
    ];
    NAMES.contains(&f)
}

/// Sans Persian/Arabic fonts for headings and screens.
fn is_persian_sans(f: &str) -> bool {
    const NAMES: &[&str] = &[
        "b yekan",
        "b titr",
        "b homa",
        "b koodak",
        "b traffic",
        "b jadid",
        "b elham",
        "b kamran",
        "b davat",
        "b ferdosi",
        "yekan",
        "titr",
        "homa",
        "koodak",
        "traffic",
        "ir titr",
        "ir homa",
        "irtitr",
        "irhoma",
        "iransans",
        "iransansweb",
        "iran sans",
        "iranyekan",
        "iran yekan",
        "iranyekanweb",
        "dana",
        "estedad",
        "dubai",
        "aldhabi",
        "urdu typesetting",
        "arabic transparent",
    ];
    NAMES.contains(&f) || f.starts_with("iransans") || f.starts_with("iranyekan")
}

/// Is `family` a Thai document font (sans or serif)? A missing one is drawn in a Thai face for
/// the whole run, never in a Latin substitute (#1549). Thai-capable Latin families such as
/// Tahoma keep their Latin substitute and are classified separately
/// ([`crate::thai::is_capable_latin_family`]).
pub fn is_thai_family(family: &str) -> bool {
    crate::thai::is_document_family(family)
}

/// Is `family` a Latin family Word treats as Thai-capable (Tahoma, Microsoft Sans Serif, Arial
/// Unicode MS)? Its Latin text keeps the family's normal substitute; Thai characters use a Thai
/// face from the shared policy.
pub fn is_thai_capable_latin(family: &str) -> bool {
    crate::thai::is_capable_latin_family(family)
}

/// Khmer document fonts: the Khmer OS family, Khmer Mool1 and the like, Windows' and the Mac's
/// own and Noto's. Matched by name only.
fn is_khmer(f: &str) -> bool {
    f.starts_with("khmer")
        || f.contains("khmer")
        || matches!(f, "daunpenh" | "moolboran" | "limon" | "battambang" | "bokor" | "content" | "dangrek" | "angkor")
}

/// Hebrew document fonts.
fn is_hebrew(f: &str) -> bool {
    const NAMES: &[&str] = &["david", "miriam", "noto sans hebrew", "arial hebrew", "calibri hebrew"];
    NAMES.contains(&f) || f.contains("hebrew")
}

/// The first installed substitute for a missing `family`. A Thai document family resolves through
/// the shared ordered policy ([`crate::thai`]); it prefers a face with Latin letters and digits as
/// well as Thai ones (a Thai face without Latin would leave a mixed run in two fonts, #1549), then
/// one with Thai letters, then the first installed. `None` when the whole policy is absent, so the
/// caller can degrade to the coverage search.
fn substitute(db: &FontDb, family: &str) -> Option<&'static str> {
    if let Some(role) = crate::thai::role(family)
        && role != crate::thai::Role::CapableLatin
    {
        return crate::thai::substitute_for(role, crate::thai::current_platform(), db, &[&['\u{0E19}', 'A', 'z', '0', '9'], &['\u{0E19}']]);
    }
    substitutes(family).iter().copied().find(|s| db.has_family(s))
}

fn style_name(bold: bool, italic: bool) -> &'static str {
    match (bold, italic) {
        (false, false) => "Regular",
        (true, false) => "Bold",
        (false, true) => "Italic",
        (true, true) => "Bold Italic",
    }
}

type Memo = (u64, HashMap<(String, bool, bool), Resolved>);

fn resolve_cache() -> &'static Mutex<Memo> {
    static CACHE: OnceLock<Mutex<Memo>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new((0, HashMap::new())))
}

const FAMILY_CACHE_MAX: usize = 4096;
/// Family names longer than this (bytes) are resolved but never remembered.
const FAMILY_KEY_MAX: usize = 256;

/// Resolve `family` with bold/italic to a face (memoised until user fonts are added or removed,
/// [`FontDb::generation`]).
pub fn resolve(family: &str, bold: bool, italic: bool) -> Resolved {
    let cache = resolve_cache();
    let db = FontDb::global();
    let generation = db.generation();
    let cacheable = family.len() <= FAMILY_KEY_MAX;
    let key = (if cacheable { family.to_string() } else { String::new() }, bold, italic);
    {
        let mut c = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if c.0 != generation {
            *c = (generation, HashMap::new());
        }
        if cacheable && let Some(r) = c.1.get(&key) {
            return *r;
        }
    }
    let r = resolve_in(db, family, bold, italic);
    let mut c = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if cacheable && c.0 == generation {
        if c.1.len() >= FAMILY_CACHE_MAX {
            c.1.clear();
        }
        c.1.insert(key, r);
    }
    r
}

/// [`resolve`] against a given database, without the memo (the seam tests and embedders use).
fn resolve_in(db: &FontDb, family: &str, bold: bool, italic: bool) -> Resolved {
    // Printer-driver Symbol aliases use the same encoding as Symbol.
    let base = if family.trim().eq_ignore_ascii_case("Symbol (PCL6)") { "Symbol" } else { charset_base_family(family) };
    let style = style_name(bold, italic);
    let (face, substituted) = if db.has_family(family) {
        (db.face(family, style), false)
    } else if base != family && db.has_family(base) {
        // Windows maps the charset names to the font itself: not a substitution.
        (db.face(base, style), false)
    } else if let Some(sub) = substitute(db, base) {
        (db.face(sub, style), true)
    } else if is_thai_family(base) {
        // No face in the ordered Thai policy is installed. Degrade to the font database's
        // existing coverage search for a Thai letter instead of failing hard (#15).
        (db.fallback_for('\u{0E19}', 0).unwrap_or_else(|| db.face(crate::FALLBACK_FAMILY, style)), true)
    } else {
        (db.face(crate::FALLBACK_FAMILY, style), true)
    };
    Resolved { synth_bold: bold && face.weight < 600.0, synth_italic: italic && !face.italic, substituted, face: FaceRef::of(&face) }
}

/// A line's (ascent, descent) from hhea values, both positive: the line gap goes above the ascent.
fn hhea_line(ascender: i16, descender: i16, line_gap: i16) -> (f64, f64) {
    (f64::from(ascender) + f64::from(line_gap.max(0)), f64::from(descender).abs())
}

/// Line metrics for a face in font units: (ascent, descent), both positive, the way word
/// processors measure single line spacing: the hhea values with the line gap above the ascent
/// (Word puts the first baseline that far below the line top), falling back to OS/2
/// usWinAscent/usWinDescent.
pub fn line_metrics(face: &crate::FontFace) -> (f64, f64) {
    static CACHE: OnceLock<Mutex<HashMap<u32, (f64, f64)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(v) = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&face.id()) {
        return *v;
    }
    let mut v = (face.ascent.max(1.0), face.descent.abs());
    if let Some(f) = face.skrifa() {
        // hhea ascender + descender + line gap (Word on macOS), the gap above the ascent; OS/2
        // win metrics only when the hhea values are missing, and never more than twice the em.
        let hhea = f.hhea().ok().map(|h| hhea_line(h.ascender().to_i16(), h.descender().to_i16(), h.line_gap().to_i16()));
        let win = f.os2().ok().map(|o| (o.us_win_ascent() as f64, o.us_win_descent() as f64));
        let ok = |m: &(f64, f64)| m.0 > 0.0 && m.0 + m.1 <= face.upem * 2.0;
        if let Some(m) = hhea.filter(ok) {
            v = m;
        } else if let Some(m) = win.filter(ok) {
            v = m;
        }
    }
    cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(face.id(), v);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_gap_goes_above_the_ascent() {
        // Times New Roman (hhea 1825 / -443, gap 87), and a font with a large gap: Word puts the
        // first baseline the ascent plus the whole gap below the line top.
        assert_eq!(hhea_line(1825, -443, 87), (1912.0, 443.0));
        assert_eq!(hhea_line(1536, -512, 452), (1988.0, 512.0));
        // A negative gap adds nothing; the most negative descender doesn't overflow.
        assert_eq!(hhea_line(1000, -300, -50), (1000.0, 300.0));
        assert_eq!(hhea_line(1000, i16::MIN, 0), (1000.0, 32768.0));
    }

    #[test]
    fn family_cache_stays_bounded() {
        for i in 0..(FAMILY_CACHE_MAX + 50) {
            let _ = resolve(&format!("Zz Missing Family {i}"), false, false);
        }
        assert!(resolve_cache().lock().unwrap_or_else(|e| e.into_inner()).1.len() <= FAMILY_CACHE_MAX);
        let long = "L".repeat(10_000);
        let _ = resolve(&long, false, false);
        assert!(!resolve_cache().lock().unwrap_or_else(|e| e.into_inner()).1.keys().any(|k| k.0.len() > 1000));
    }

    #[test]
    fn unknown_family_substitutes() {
        let r = resolve("Definitely Not A Font 123", false, false);
        assert!(r.substituted);
        let b = resolve("Source Serif 4", true, false);
        assert!(!b.substituted);
        assert!(!b.synth_bold);
        let (a, d) = line_metrics(&b.face);
        assert!(a > 0.0 && d >= 0.0);
    }

    #[test]
    fn palatino_family_prefers_palatino_look_alikes_over_generic_serif() {
        for name in ["Palatino Linotype", "Book Antiqua", "palatino"] {
            let list = substitutes(name);
            let pos = |n: &str| list.iter().position(|x| *x == n);
            assert!(pos("Palatino").is_some() && pos("TeX Gyre Pagella").is_some() && pos("P052").is_some(), "{name}");
            // All of them come before the generic serif fallbacks (Caladea, Times New Roman).
            assert!(pos("Palatino") < pos("Caladea") && pos("TeX Gyre Pagella") < pos("Times New Roman"), "{name}");
        }
        // Other serif families are unchanged.
        assert_eq!(substitutes("Georgia").first(), Some(&"Caladea"));
        // Charset aliases are the font itself, so they substitute like it.
        assert_eq!(charset_base_family("Times New Roman Cyr"), "Times New Roman");
        assert_eq!(charset_base_family("Arial CYR"), "Arial");
        assert_eq!(charset_base_family("Courier New CE"), "Courier New");
        assert_eq!(charset_base_family("Arial (Hebrew)"), "Arial");
        assert_eq!(charset_base_family("Cyr"), "Cyr");
        assert_eq!(charset_base_family("Calibri"), "Calibri");
        assert_eq!(substitutes(charset_base_family("Times New Roman Cyr")), substitutes("Times New Roman"));
    }

    #[test]
    fn pcl6_symbol_has_symbols_line_metrics() {
        assert_eq!(symbol_font_metrics("Symbol (PCL6)"), symbol_font_metrics("Symbol"));
        assert_eq!(symbol_font_metrics("Arial"), None);
    }

    #[test]
    fn a_one_em_symbol_face_takes_windows_symbols_line() {
        // Apple's Symbol (2048 units): hhea 1436 / -612, no gap, one em; Windows' Symbol line is 1.221 em.
        assert!(one_em(hhea_line(1436, -612, 0), 2048.0));
        assert!(!one_em((1005.0, 216.0), 1000.0));
        assert!(!one_em((899.0, 211.0), 1000.0));
        assert!(!one_em((0.0, 0.0), 0.0));
        let arial = resolve("Arial", false, false);
        assert_eq!(symbol_line("Arial", &arial), None);
        let symbol = resolve("Symbol", false, false);
        if symbol.substituted {
            assert_eq!(symbol_line("Symbol", &symbol), symbol_font_metrics("Symbol"));
        }
    }

    #[test]
    fn pcl6_symbol_resolves_as_symbol() {
        let actual = resolve("Symbol (PCL6)", false, false);
        let db = FontDb::global();
        if db.has_family("Symbol (PCL6)") {
            assert_eq!(actual.face.id(), db.face("Symbol (PCL6)", "Regular").id());
            assert!(!actual.substituted);
            return;
        }
        let expected = resolve("Symbol", false, false);
        assert_eq!(actual.face.id(), expected.face.id());
        assert_eq!(actual.substituted, expected.substituted);
    }

    #[test]
    fn unicode_arial_is_proportional() {
        for name in ["Arial Unicode MS", "ARIAL UNICODE MS"] {
            let candidates = substitutes(name);
            assert_eq!(candidates.first(), Some(&"Arial"));
            assert_eq!(&candidates[1..], substitutes("Arial"));
        }
        assert_eq!(substitutes("Cascadia Code").first(), Some(&"Courier New"));
    }

    #[test]
    fn symbol_fonts_substitute_symbol_faces() {
        for f in ["Symbol", "Wingdings", "Wingdings 2", "Wingdings 3", "Webdings"] {
            assert!(substitutes(f).contains(&"Noto Sans Symbols 2"), "{f}");
        }
    }

    #[test]
    fn resolves_weights_named_as_their_own_family() {
        // Word stores a Medium face as the family "Inter Medium" (#219).
        let r = resolve("Inter Medium", false, false);
        assert!(!r.substituted && !r.synth_bold && !r.synth_italic);
        assert_eq!((r.face.family.as_str(), r.face.style.as_str()), ("Inter", "Medium"));
        // Bold and italic within it are drawn from the Medium face.
        let b = resolve("Inter Medium", true, true);
        assert_eq!(b.face.style, "Medium");
        assert!(!b.substituted && b.synth_bold && b.synth_italic);
    }

    #[test]
    fn synthesizes_missing_styles() {
        // JetBrains Mono ships only Regular here.
        let r = resolve("JetBrains Mono", true, true);
        assert!(r.synth_bold && r.synth_italic);
    }

    #[test]
    fn hebrew_fonts_substitute_hebrew_faces() {
        for f in ["David", "Miriam", "Arial Hebrew", "Calibri Hebrew", "Noto Sans Hebrew"] {
            assert!(substitutes(f).contains(&"Noto Sans Hebrew"), "{f}");
            // David is a Windows font; only the Windows list offers it.
            if cfg!(target_os = "windows") {
                assert!(substitutes(f).contains(&"David") || f == "David", "{f}");
            }
        }
        // Generic names with "Hebrew" in them also get Hebrew substitutions
        assert!(substitutes("SomeFont Hebrew").contains(&"Noto Sans Hebrew"));
    }

    #[test]
    fn khmer_fonts_substitute_khmer_faces() {
        for f in ["Khmer OS Bokor", "Khmer OS Battambang", "Khmer Mool1", "Khmer OS Muol Light"] {
            assert!(substitutes(f).iter().any(|s| s.contains("Khmer")), "{f}: {:?}", substitutes(f));
        }
    }

    #[test]
    fn thai_fonts_substitute_thai_faces() {
        // Every Thai document family maps to a list of Thai faces, never to the Latin lists.
        for f in [
            "TH Sarabun New",
            "TH SarabunPSK",
            "Angsana New",
            "AngsanaUPC",
            "Cordia New",
            "CordiaUPC",
            "Browallia New",
            "BrowalliaUPC",
            "JasmineUPC",
            "KodchiangUPC",
            "LilyUPC",
            "DilleniaUPC",
            "EucrosiaUPC",
            "FreesiaUPC",
            "IrisUPC",
            "Leelawadee",
            "Leelawadee UI",
            "Noto Sans Thai",
            "TH Niramit AS",
        ] {
            assert!(is_thai_family(f), "{f}");
            let list = substitutes(f);
            assert!(list.iter().any(|s| s.contains("Thai")), "{f}: {list:?}");
            assert!(!list.contains(&"Carlito") && !list.contains(&"Caladea"), "{f}: {list:?}");
        }
        // Serif Thai families prefer a serif Thai face; the sans families a sans one.
        let pos = |family: &str, name: &str| substitutes(family).iter().position(|s| *s == name).unwrap_or(usize::MAX);
        assert!(pos("Angsana New", "Noto Serif Thai") < pos("Angsana New", "Noto Sans Thai"));
        assert!(pos("TH Sarabun New", "Noto Sans Thai") < pos("TH Sarabun New", "Noto Serif Thai"));
        // Latin and other script families are unchanged.
        assert!(!is_thai_family("Arial") && !is_thai_family("Tahoma") && !is_thai_family("Calibri") && !is_thai_family("David"));
        assert_eq!(substitutes("Calibri").first(), Some(&"Carlito"));
    }

    #[test]
    fn a_missing_thai_font_resolves_to_an_installed_thai_face() {
        // The issue's run (#1549): Thai letters and digits, Latin letters, digits and the baht
        // sign in one run of a Thai font that isn't installed.
        const RUN: &str = "[D] น้ำ ค่ำ ปั้น ฝั่ง ปู่ ๑๒๓๔๕๖๗๘๙๐ ฿1,250.50";
        let db = FontDb::global();
        for family in ["TH Sarabun New", "Angsana New", "Cordia New", "BrowalliaUPC"] {
            if db.has_family(family) {
                continue;
            }
            let r = resolve(family, false, false);
            let face = r.face.get();
            if !face.covers('\u{0E19}') {
                eprintln!("skipped {family}: no Thai face from its substitute list is installed");
                continue;
            }
            assert!(r.substituted, "{family}");
            let thai: Vec<char> = RUN.chars().filter(|c| ('\u{0E00}'..='\u{0E7F}').contains(c) && !face.covers(*c)).collect();
            assert!(thai.is_empty(), "{family} → {} lacks {thai:?}", face.family);
            // A listed Thai face with Latin letters and digits wins over one without them.
            let latin = |name: &str| {
                let f = db.face(name, "Regular");
                f.named(name) && f.covers('\u{0E19}') && f.covers('D') && f.covers('5')
            };
            if substitutes(family).iter().any(|s| db.has_family(s) && latin(s)) {
                let missing: Vec<char> = RUN.chars().filter(|c| !c.is_whitespace() && !face.covers(*c)).collect();
                assert!(missing.is_empty(), "{family} → {} lacks {missing:?}", face.family);
            }
        }
    }

    #[test]
    fn thai_capable_latin_families_keep_their_latin_substitutes() {
        for name in ["Tahoma", "Microsoft Sans Serif", "Arial Unicode MS"] {
            assert!(is_thai_capable_latin(name), "{name}");
            // A Latin design, not a Thai document family: a missing one is not drawn in a Thai
            // face for the whole run.
            assert!(!is_thai_family(name), "{name}");
        }
        // Their Latin text uses the ordinary Latin substitutes, not a Thai list.
        assert_eq!(substitutes("Tahoma").first(), Some(&"Carlito"));
        assert!(!substitutes("Tahoma").contains(&"TH Sarabun New"));
        assert_eq!(substitutes("Microsoft Sans Serif").first(), Some(&"Carlito"));
        assert_eq!(substitutes("Arial Unicode MS").first(), Some(&"Arial"));
        assert!(!is_thai_capable_latin("Arial") && !is_thai_capable_latin("TH Sarabun New"));
    }

    #[test]
    fn thai_serif_requests_use_serif_only_lists() {
        for name in ["Angsana New", "Browallia New", "BrowalliaUPC", "Norasi", "Kinnari", "Noto Serif Thai"] {
            assert!(is_thai_family(name), "{name}");
            let list = substitutes(name);
            assert!(list.contains(&"Noto Serif Thai"), "{name}: {list:?}");
            // Never the loopless sans designs that used to sit in the serif list.
            for sans in ["TH Sarabun New", "TH SarabunPSK", "Cordia New", "Leelawadee", "Leelawadee UI", "Tahoma", "Noto Sans Thai"] {
                assert!(!list.contains(&sans), "{name}: {list:?} holds {sans}");
            }
        }
        // The general list still starts with the two Sarabun designs, in order.
        let general = substitutes("Cordia New");
        assert_eq!(general.first(), Some(&"TH Sarabun New"));
        assert_eq!(general.get(1), Some(&"TH SarabunPSK"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_missing_thai_font_without_a_system_face_degrades_to_the_coverage_search() {
        // A database without system fonts: the policy has no installed candidate, so resolution
        // falls through the existing coverage search — to the bundled craft-fonts Thai face when
        // the build has one, otherwise to the last-resort face — instead of failing.
        let db = FontDb::with_font_dirs(Vec::new());
        db.set_system_fallback(false);
        let bundled = crate::thai_fonts().next().is_some();
        for family in ["TH Sarabun New", "Angsana New", "Cordia New", "BrowalliaUPC"] {
            let r = resolve_in(&db, family, false, false);
            assert!(r.substituted, "{family}");
            if bundled {
                assert_eq!(r.face.family, "Noto Sans Thai", "{family}");
                assert!(r.face.covers('\u{0E19}'), "the bundled Thai face draws Thai");
            } else {
                assert_eq!(r.face.family, crate::FALLBACK_FAMILY, "{family}");
                assert!(!r.face.covers('\u{0E19}'), "no bundled Thai face");
            }
        }
        // The same request against the real database still returns a usable face (Thai when the
        // system has one, the last resort otherwise): never an error or a panic.
        let r = resolve_in(FontDb::global(), "TH Sarabun New", false, false);
        assert!(r.face.upem > 0.0);
    }

    #[test]
    fn thonburi_stacks_thai_marks_through_its_apple_tables() {
        // Thonburi has `morx` but no GSUB: the shaper must still move a tone mark up over an upper
        // vowel (ปั่) instead of drawing it where it sits on a bare consonant (ป่).
        let db = FontDb::global();
        if !db.has_family("Thonburi") {
            eprintln!("skipped: Thonburi not installed");
            return;
        }
        let face = db.face("Thonburi", "Regular");
        let tone = |text: &str| crate::shape(&face, text, &[], |c| c).into_iter().last().map(|g| (g.gid, g.y_offset));
        let (bare, stacked) = (tone("\u{0E1B}\u{0E48}"), tone("\u{0E1B}\u{0E31}\u{0E48}"));
        assert!(bare.is_some() && stacked.is_some());
        assert_ne!(bare, stacked, "the tone mark over ั is raised or swapped");
    }

    #[test]
    fn fonts_with_a_clone_have_named_substitutes() {
        for f in ["Times New Roman", "Times New Roman Cyr", "Arial", "Courier New", "Palatino Linotype", "Symbol"] {
            assert!(has_named_substitutes(f), "{f}");
        }
        for f in ["No Such Brand Font", "Cambria", "Georgia", "Brand Serif"] {
            assert!(!has_named_substitutes(f), "{f}");
        }
    }
}
