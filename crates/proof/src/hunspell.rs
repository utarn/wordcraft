//! Hunspell dictionaries installed on the system, read at runtime.
//!
//! WordCraft doesn't bundle large spelling dictionaries. For languages with an affix dictionary
//! (Polish, Russian), it reads the standard Hunspell `.aff`/`.dic` pair installed by the operating system,
//! a package manager or LibreOffice, with [`crate::affix::AffixDict::from_hunspell`]. When none is
//! installed the language has no spelling check (no marks, no errors), like Word without the
//! language's proofing tools. Thai's system dictionary is TIS-620 encoded; [`decode`] handles
//! that alongside UTF-8 and the single-byte encodings older European dictionaries use.
//!
//! Search order: the directory in `WORDCRAFT_DICTIONARIES` (or one set with
//! [`set_dictionary_dir`]), then the usual system locations. On the web there are none.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::affix::AffixDict;

/// Largest `.aff` or `.dic` file read (real ones are a few MB).
const MAX_FILE: u64 = 64 << 20;

/// An extra directory searched first (a setting), set by the front end.
static EXTRA_DIR: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Search `dir` before the system locations (a "dictionaries folder" setting); `None` clears it.
/// Takes effect for dictionaries not loaded yet.
pub fn set_dictionary_dir(dir: Option<PathBuf>) {
    *EXTRA_DIR.write().unwrap_or_else(|e| e.into_inner()) = dir;
}

/// The folder set with [`set_dictionary_dir`], for tests that change it and must restore it.
#[cfg(test)]
pub(crate) fn dictionary_dir() -> Option<PathBuf> {
    EXTRA_DIR.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The LibreOffice extension directories a Hunspell dictionary name may live in: LibreOffice's
/// IDs are per dictionary — Polish is `dict-pl`, Brazilian Portuguese `dict-pt-BR` — so both
/// the primary-language folder and the dashed full name are tried.
pub(crate) fn lo_ext_dirs(name: &str) -> Vec<String> {
    let primary = name.split(['-', '_']).next().unwrap_or(name);
    let dashed = name.replace('_', "-");
    let first = format!("dict-{primary}");
    let full = format!("dict-{dashed}");
    if full == first { vec![first] } else { vec![first, full] }
}

/// Directories the user chose for Hunspell dictionaries, most specific first: the folder set with
/// [`set_dictionary_dir`] ("Dictionaries folder"), then `WORDCRAFT_DICTIONARIES`. These are
/// searched before the system locations. The web has no filesystem, so only the setting (if a
/// front end set one) is returned there.
pub fn user_search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = EXTRA_DIR.read().unwrap_or_else(|e| e.into_inner()).clone() {
        dirs.push(d);
    }
    if cfg!(target_arch = "wasm32") {
        return dirs;
    }
    if let Some(d) = std::env::var_os("WORDCRAFT_DICTIONARIES").filter(|v| !v.is_empty()) {
        dirs.push(PathBuf::from(d));
    }
    dirs
}

/// The usual operating-system, package-manager and LibreOffice locations, most specific first.
/// `lo_exts`, when a language is being searched for, adds LibreOffice's extension folders for it
/// (`dict-pt-BR`). Empty on the web.
pub fn system_search_dirs(lo_exts: Option<&[String]>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if cfg!(target_arch = "wasm32") {
        return dirs;
    }
    // LibreOffice keeps each language under `…/share/extensions/<dict-xx>`.
    let lo_dirs =
        |prefix: &str| -> Vec<PathBuf> { lo_exts.map_or_else(Vec::new, |exts| exts.iter().map(|e| PathBuf::from(prefix).join(e)).collect()) };
    let home = std::env::var_os("HOME").filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        for var in ["LOCALAPPDATA", "APPDATA"] {
            if let Some(base) = std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from) {
                dirs.push(base.join("WordCraft").join("dictionaries"));
                dirs.push(base.join("hunspell"));
            }
        }
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(base) = std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from) {
                dirs.extend(lo_dirs(&base.join("LibreOffice").join("share").join("extensions").to_string_lossy()));
            }
        }
        // If LibreOffice is installed on another drive (e.g. D:\Program Files\LibreOffice), check drives C..Z
        for drive in b'C'..=b'Z' {
            let drive_char = drive as char;
            for pf in ["Program Files", "Program Files (x86)"] {
                let lo_base = PathBuf::from(format!(r"{drive_char}:\{pf}\LibreOffice\share\extensions"));
                if lo_base.is_dir() {
                    dirs.extend(lo_dirs(&lo_base.to_string_lossy()));
                }
            }
        }
    } else if cfg!(target_os = "macos") {
        if let Some(h) = &home {
            dirs.push(h.join("Library/Spelling"));
        }
        dirs.push(PathBuf::from("/Library/Spelling"));
        dirs.push(PathBuf::from("/opt/homebrew/share/hunspell"));
        dirs.push(PathBuf::from("/usr/local/share/hunspell"));
        dirs.extend(lo_dirs("/Applications/LibreOffice.app/Contents/Resources/extensions"));
    } else {
        if let Some(h) = &home {
            dirs.push(h.join(".local/share/hunspell"));
        }
        for d in ["/usr/share/hunspell", "/usr/share/myspell", "/usr/share/myspell/dicts", "/usr/local/share/hunspell", "/usr/local/share/myspell"] {
            dirs.push(PathBuf::from(d));
        }
        for d in ["/usr/lib/libreoffice/share/extensions", "/opt/libreoffice/share/extensions"] {
            dirs.extend(lo_dirs(d));
        }
    }
    dirs
}

/// Directories that may hold Hunspell dictionaries, most specific first: the user's folders,
/// then the system locations. `lo_exts`, when a language is being searched for, adds
/// LibreOffice's extension folders for it (`dict-pt-BR`).
pub fn search_dirs(lo_exts: Option<&[String]>) -> Vec<PathBuf> {
    let mut dirs = user_search_dirs();
    dirs.extend(system_search_dirs(lo_exts));
    dirs
}

/// The first `<name>.aff`/`<name>.dic` pair found for any of `names` (`pl_PL`, `pl`), searching
/// [`search_dirs`] (including LibreOffice's extension folder for the first name).
pub fn find(names: &[&str]) -> Option<(PathBuf, PathBuf)> {
    let lo = names.first().copied().map(lo_ext_dirs);
    search_dirs(lo.as_deref()).into_iter().find_map(|dir| find_in(&dir, names))
}

/// The first `<name>.aff`/`<name>.dic` pair for any of `names` in `dir`.
pub fn find_in(dir: &Path, names: &[&str]) -> Option<(PathBuf, PathBuf)> {
    names.iter().find_map(|n| {
        let aff = dir.join(format!("{n}.aff"));
        let dic = dir.join(format!("{n}.dic"));
        (aff.is_file() && dic.is_file()).then_some((aff, dic))
    })
}

/// Read a Hunspell pair from disk (size-capped), decoded per the `.aff`'s `SET` line.
pub fn load(aff: &Path, dic: &Path) -> Result<AffixDict, String> {
    let (aff_text, dic_text) = read_decoded(aff, dic)?;
    let source = format!("Hunspell dictionary {}", dic.display());
    AffixDict::from_hunspell(&aff_text, &dic_text, &[source.as_str()])
}

/// Read a Hunspell `.aff`/`.dic` pair from disk (each file capped at [`MAX_FILE`]) and decode
/// both to UTF-8 from the encoding the `.aff`'s `SET` line names. A directory, a missing file or
/// a file over the cap is an error, never a panic.
pub fn read_decoded(aff: &Path, dic: &Path) -> Result<(String, String), String> {
    let read = |p: &Path| -> Result<Vec<u8>, String> {
        let len = std::fs::metadata(p).map_err(|e| format!("{}: {e}", p.display()))?.len();
        if len > MAX_FILE {
            return Err(format!("{}: too large ({len} bytes)", p.display()));
        }
        std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))
    };
    decode(&read(aff)?, &read(dic)?)
}

/// ISO 8859-2 code points 0xA0..=0xFF (0x00..=0x9F map to themselves).
const ISO_8859_2_HIGH: &str = "\u{a0}Ą˘Ł¤ĽŚ§¨ŠŞŤŹ\u{ad}ŽŻ°ą˛ł´ľśˇ¸šşťź˝žżŔÁÂĂÄĹĆÇČÉĘËĚÍÎĎĐŃŇÓÔŐÖ×ŘŮÚŰÜÝŢßŕáâăäĺćçčéęëěíîďđńňóôőö÷řůúűüýţ˙";

fn decode_latin2(bytes: &[u8]) -> String {
    let high: Vec<char> = ISO_8859_2_HIGH.chars().collect();
    bytes.iter().map(|&b| if b < 0xa0 { char::from(b) } else { high.get(usize::from(b - 0xa0)).copied().unwrap_or('\u{fffd}') }).collect()
}

/// KOI8-R code points 0x80..=0xFF (0x00..=0x7F are ASCII): the encoding of older Russian
/// dictionaries.
const KOI8_R_HIGH: &str =
    "─│┌┐└┘├┤┬┴┼▀▄█▌▐░▒▓⌠■∙√≈≤≥\u{a0}⌡°²·÷═║╒ё╓╔╕╖╗╘╙╚╛╜╝╞╟╠╡Ё╢╣╤╥╦╧╨╩╪╫╬©юабцдефгхийклмнопярстужвьызшэщчъЮАБЦДЕФГХИЙКЛМНОПЯРСТУЖВЬЫЗШЭЩЧЪ";

/// Windows-1251 code points 0x80..=0xFF (0x98 is unassigned): the other one Russian dictionaries
/// use.
const CP1251_HIGH: &str = "ЂЃ‚ѓ„…†‡€‰Љ‹ЊЌЋЏђ‘’“”•–—\u{fffd}™љ›њќћџ\u{a0}ЎўЈ¤Ґ¦§Ё©Є«¬\u{ad}®Ї°±Ііґµ¶·ё№є»јЅѕїАБВГДЕЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдежзийклмнопрстуфхцчшщъыьэюя";

/// Decode a single-byte encoding whose bytes 0x80..=0xFF are `high` (ASCII below).
fn decode_high(bytes: &[u8], high: &str) -> String {
    let high: Vec<char> = high.chars().collect();
    bytes.iter().map(|&b| if b < 0x80 { char::from(b) } else { high.get(usize::from(b - 0x80)).copied().unwrap_or('\u{fffd}') }).collect()
}

/// Decode TIS-620 (the encoding of the `th_TH` proofing dictionary): ASCII keeps its code, Thai
/// occupies 0xA1..=0xDA and 0xDF..=0xFB contiguously from U+0E01 (0xDF is the baht sign,
/// U+0E3F), 0xA0 is a no-break space and the reserved bytes become U+FFFD. The mapping is
/// monotonic enough to compute, so there is no table to get wrong.
fn decode_tis620(bytes: &[u8]) -> String {
    const FIRST: u32 = 0x0E01;
    bytes
        .iter()
        .map(|&b| match b {
            0x00..=0x7F => char::from(b),
            0xA0 => '\u{a0}',
            0xA1..=0xDA | 0xDF..=0xFB => char::from_u32(FIRST.saturating_add(u32::from(b) - 0xA1)).unwrap_or('\u{fffd}'),
            _ => '\u{fffd}',
        })
        .collect()
}

/// Decode an `.aff`/`.dic` pair to UTF-8 from the encoding the `.aff`'s `SET` line names
/// (UTF-8 when there is none; ISO 8859-2 is one Polish dictionaries use, KOI8-R and
/// Windows-1251 are ones older Russian dictionaries use, TIS-620 the Thai system dictionary).
pub fn decode(aff: &[u8], dic: &[u8]) -> Result<(String, String), String> {
    let head = String::from_utf8_lossy(aff.get(..aff.len().min(8192)).unwrap_or_default()).to_string();
    let set = head.lines().find_map(|l| l.trim_start_matches('\u{feff}').strip_prefix("SET ")).map(str::trim).unwrap_or("UTF-8").to_ascii_uppercase();
    match set.as_str() {
        "UTF-8" | "UTF8" => {
            let text = |b: &[u8], what: &str| String::from_utf8(b.to_vec()).map_err(|e| format!("{what}: not UTF-8: {e}"));
            Ok((text(aff, ".aff")?, text(dic, ".dic")?))
        }
        "ISO8859-2" | "ISO-8859-2" => Ok((decode_latin2(aff), decode_latin2(dic))),
        "KOI8-R" | "KOI8R" => Ok((decode_high(aff, KOI8_R_HIGH), decode_high(dic, KOI8_R_HIGH))),
        "CP1251" | "WINDOWS-1251" | "MICROSOFT-CP1251" => Ok((decode_high(aff, CP1251_HIGH), decode_high(dic, CP1251_HIGH))),
        "TIS620" | "TIS-620" | "TIS620-2533" => Ok((decode_tis620(aff), decode_tis620(dic))),
        other => Err(format!("unsupported encoding {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_latin2_and_utf8() {
        let aff = b"SET ISO8859-2\nTRY a\n";
        let (a, d) = decode(aff, b"2\n\xbf\xf3\xb3w\n").unwrap();
        assert!(a.contains("TRY a"));
        assert_eq!(d, "2\nżółw\n");
        let (_, d) = decode("SET UTF-8\n".as_bytes(), "żółw".as_bytes()).unwrap();
        assert_eq!(d, "żółw");
        assert!(decode(b"SET SHIFT_JIS\n", b"").is_err());
        assert!(decode(b"SET UTF-8\n", b"\xff\xfe").is_err());
    }

    #[test]
    fn decodes_russian_single_byte_encodings() {
        // `ёлка` in KOI8-R (0xA3 0xCC 0xCB 0xC1) and in Windows-1251 (0xB8 0xEB 0xEA 0xE0).
        let (a, d) = decode(b"SET KOI8-R\nTRY \xcf\xc5\n", b"1\n\xa3\xcc\xcb\xc1/A\n").unwrap();
        assert!(a.contains("TRY ое"), "{a}");
        assert_eq!(d, "1\nёлка/A\n");
        let (_, d) = decode(b"SET microsoft-cp1251\n", b"1\n\xb8\xeb\xea\xe0 \xc4\xee\xec\n").unwrap();
        assert_eq!(d, "1\nёлка Дом\n");
        assert_eq!(decode_high(&[0x98], CP1251_HIGH), "\u{fffd}");
    }

    #[test]
    fn decodes_tis620_thai_dictionaries() {
        // TIS-620: ASCII as itself, Thai at 0xA1.. (U+0E01..), 0xDF the baht sign.
        let dic = [b'3', b'\n', 0xA1, 0xE7, b'\n', 0xB7, b'\n', 0xDF, b'\n'];
        let (_, d) = decode(b"SET TIS620\n", &dic).unwrap();
        assert_eq!(d, "3\nก็\nท\n฿\n");
        // Reserved bytes become U+FFFD; decoding is total and never panics.
        assert_eq!(super::decode_tis620(&[0x80, 0x9F, 0xDB, 0xDE, 0xFC, 0xFF]), "\u{fffd}\u{fffd}\u{fffd}\u{fffd}\u{fffd}\u{fffd}");
        assert_eq!(super::decode_tis620(&[0xA0]), "\u{a0}");
        for set in ["SET TIS-620\n", "SET tis620-2533\n"] {
            assert!(decode(set.as_bytes(), b"1\n\xA1\n").is_ok(), "{set:?}");
        }
    }

    #[test]
    fn refuses_oversized_files_before_reading_them() {
        let dir = std::env::temp_dir().join(format!("wordcraft-hunspell-big-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let aff = dir.join("big.aff");
        let dic = dir.join("big.dic");
        let f = std::fs::File::create(&aff).unwrap();
        f.set_len(MAX_FILE.saturating_add(1)).unwrap();
        std::fs::write(&dic, "1\ncat\n").unwrap();
        assert!(read_decoded(&aff, &dic).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn libreoffice_extension_dirs_follow_the_language() {
        // LibreOffice's IDs are per dictionary: Polish is `dict-pl`, Brazilian Portuguese
        // `dict-pt-BR`, so both the primary-language folder and the dashed full name are tried.
        assert_eq!(super::lo_ext_dirs("cs_CZ"), ["dict-cs", "dict-cs-CZ"]);
        assert_eq!(super::lo_ext_dirs("id_ID"), ["dict-id", "dict-id-ID"]);
        assert_eq!(super::lo_ext_dirs("pl_PL"), ["dict-pl", "dict-pl-PL"]);
        assert_eq!(super::lo_ext_dirs("pt_BR"), ["dict-pt", "dict-pt-BR"]);
        assert_eq!(super::lo_ext_dirs("de-DE"), ["dict-de", "dict-de-DE"]);
        let dirs = search_dirs(Some(&super::lo_ext_dirs("pt_BR")));
        assert!(dirs.iter().any(|d| d.ends_with("dict-pt-BR")), "{dirs:?}");
        assert!(search_dirs(None).iter().all(|d| !d.to_string_lossy().contains("dict-")), "no language: no LibreOffice guess");
    }

    #[test]
    fn finds_and_loads_a_pair_in_a_directory() {
        let dir = std::env::temp_dir().join(format!("wordcraft-hunspell-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(find_in(&dir, &["xx_XX"]).is_none(), "no files: nothing found");
        std::fs::write(dir.join("xx_XX.aff"), "SET UTF-8\nSFX A Y 1\nSFX A 0 y .\n").unwrap();
        assert!(find_in(&dir, &["xx_XX"]).is_none(), "an .aff alone isn't a dictionary");
        std::fs::write(dir.join("xx_XX.dic"), "1\nkot/A\n").unwrap();
        let (aff, dic) = find_in(&dir, &["yy", "xx_XX"]).unwrap();
        let d = load(&aff, &dic).unwrap();
        assert!(d.check("kot") && d.check("koty") && !d.check("kotx"));
        assert!(load(&dir.join("missing.aff"), &dic).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
