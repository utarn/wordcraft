//! Compresses the vendored PyThaiNLP Thai word list (`assets/spelling/th-words.txt`, CC0-1.0)
//! into `OUT_DIR/th-words.bin`. The runtime embeds the bytes and decompresses them once, so the
//! Thai spelling dictionary needs no filesystem and works on the web.
//!
//! The source list is vendored unmodified (its SHA-256 is in `ATTRIBUTION.md`). Entries are
//! validated and capped at runtime, where a corrupt or hostile payload can only turn Thai
//! spelling off, never panic. Any build failure is a build error: a malformed asset must never
//! become a silently empty dictionary.

use std::io::Write as _;
use std::path::PathBuf;

/// The word list is ~1.5 MB; cap it so a corrupted asset cannot exhaust memory at build time.
const MAX_SOURCE_BYTES: usize = 16 << 20;

fn main() {
    println!("cargo:rerun-if-changed=../../assets/spelling/th-words.txt");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("th-words.bin");
    if let Err(e) = build(&out) {
        // A build script has no `?`; report through cargo and fail the build.
        println!("cargo:error={e}");
        std::process::exit(1);
    }
}

fn build(out: &PathBuf) -> Result<(), String> {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").ok_or("CARGO_MANIFEST_DIR unset")?);
    let asset = manifest.join("../../assets/spelling/th-words.txt");
    let raw = std::fs::read(&asset).map_err(|e| format!("{}: {e}", asset.display()))?;
    if raw.is_empty() {
        return Err(format!("{}: empty word list", asset.display()));
    }
    if raw.len() > MAX_SOURCE_BYTES {
        return Err(format!("{}: larger than {MAX_SOURCE_BYTES} bytes", asset.display()));
    }
    std::str::from_utf8(&raw).map_err(|e| format!("{}: not UTF-8: {e}", asset.display()))?;
    let compressed = miniz_oxide::deflate::compress_to_vec(&raw, 10);
    let mut file = std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    file.write_all(&compressed).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(())
}
