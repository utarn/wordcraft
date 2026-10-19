//! Compiles the vendored ICU Thai word list (`assets/thai/thaidict.txt`, Unicode-3.0) into an
//! FST in `OUT_DIR`. The runtime embeds the bytes, so the web build reads no files.
//!
//! The dictionary is vendored unmodified. Here the BOM and comment lines are stripped, entries
//! are validated and capped, and the sorted, duplicate-free list is compiled. Any failure is a
//! build error: a malformed asset must never become a silently empty dictionary.

use std::io::Write as _;
use std::path::PathBuf;

/// A word longer than this (codepoints) is rejected: the runtime never looks past it.
const MAX_WORD_CODEPOINTS: usize = 55;
/// The word list is ~26k entries; cap it so a corrupted asset cannot exhaust memory.
const MAX_ENTRIES: usize = 500_000;

fn main() {
    println!("cargo::rerun-if-changed=../../assets/thai/thaidict.txt");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("thaidict.fst");
    if let Err(e) = build(&out) {
        // A build script has no `?`; report through cargo and fail the build.
        println!("cargo::error={e}");
        std::process::exit(1);
    }
}

fn build(out: &PathBuf) -> Result<(), String> {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").ok_or("CARGO_MANIFEST_DIR unset")?);
    let asset = manifest.join("../../assets/thai/thaidict.txt");
    let raw = std::fs::read(&asset).map_err(|e| format!("{}: {e}", asset.display()))?;
    let text = std::str::from_utf8(&raw).map_err(|e| format!("{}: not UTF-8: {e}", asset.display()))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut entries: Vec<Vec<u8>> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let word = line.trim();
        if word.is_empty() || word.starts_with('#') {
            continue;
        }
        if entries.len() >= MAX_ENTRIES {
            return Err(format!("{}: more than {MAX_ENTRIES} entries", asset.display()));
        }
        validate(word).map_err(|e| format!("{}:{}: {e}", asset.display(), n + 1))?;
        entries.push(word.as_bytes().to_vec());
    }
    if entries.is_empty() {
        return Err(format!("{}: no entries", asset.display()));
    }
    entries.sort_unstable();
    entries.dedup();
    let mut builder = fst::SetBuilder::memory();
    for word in &entries {
        builder.insert(word).map_err(|e| format!("FST: {e}"))?;
    }
    let bytes = builder.into_inner().map_err(|e| format!("FST: {e}"))?;
    let mut file = std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    file.write_all(&bytes).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(())
}

/// An entry is one run of Thai word characters with no spaces or controls.
fn validate(word: &str) -> Result<(), String> {
    let count = word.chars().count();
    if count > MAX_WORD_CODEPOINTS {
        return Err(format!("entry longer than {MAX_WORD_CODEPOINTS} codepoints"));
    }
    for c in word.chars() {
        let u = u32::from(c);
        if !matches!(u, 0x0E01..=0x0E3A | 0x0E3F | 0x0E40..=0x0E4E | 0x0E50..=0x0E5B) {
            return Err(format!("entry has non-Thai character U+{u:04X}"));
        }
    }
    Ok(())
}
