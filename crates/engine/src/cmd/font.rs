//! Fonts the user adds without rebuilding (#582): `font.add`, `font.list`, `font.remove`.
//!
//! An added font joins the process-wide font database on top of the bundled and system fonts
//! (never replacing a face already there), so it shows in the font lists, lays out, renders and
//! embeds in PDFs like any other. Where it is kept between runs is [`FontDb::load_user_fonts`]'s
//! folder (desktop); without one it lasts for the session (the web shell stores it itself).

use serde_json::{Value, json};
use wordcraft_doc::StoryRef;
use wordcraft_fonts::{FontDb, UserFont};

use crate::cmd::insert::base64_decode;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("font.add", "Add Font…", "Home › Font", add)
            .params(r#"{"path"?: string, "data"?: base64, "name"?: string (file name for "data")} — a TTF, OTF, TTC, WOFF or WOFF2 file; none: pick a file"#)
            .pure(),
        CommandSpec::new("font.list", "Added Fonts", "Home › Font", list).params(r#"{"all"?: bool (also every available family)}"#).pure(),
        CommandSpec::new("font.remove", "Remove Added Font", "Home › Font", remove).params(r#"{"family": string}"#).pure(),
        CommandSpec::new("font.loadSystem", "Load System Installed Fonts…", "Home › Font", load_system).pure(),
        CommandSpec::new("font.diagnostics", "Font Diagnostics", "Home › Font", diagnostics)
            .params(
                r#"{"text"?: string (default: the selection, else the whole document)} — complex-script characters no installed face draws, and document families that are not installed"#,
            )
            .pure(),
    ]
}

/// The characters a page would show as boxes and the document's missing families, for the Font
/// dialog's diagnostic: `complex` (the text has Thai or another complex script), `complexMissing`,
/// `missingChars` (`U+0E19`, …), `missingFamilies` (`{"family", "runs"}`), `canAddFont` and
/// `canLoadSystemFonts` (the UI's two remedies).
fn diagnostics(s: &mut Session, v: &Value) -> CmdResult {
    let db = FontDb::global();
    // The selection is what the Font dialog formats; with nothing selected the whole document.
    let text = p::str(v, "text")
        .map(str::to_string)
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| if s.sel.is_collapsed() { document_text(s) } else { s.selected_text() });
    // Bound the work: hostile documents and selections can be enormous.
    let text: String = text.chars().take(MAX_DIAGNOSTIC_TEXT).collect();
    let missing = wordcraft_fonts::uncovered_complex(db, &text);
    let complex = text.chars().any(wordcraft_fonts::needs_layout_tables);
    let missing_families: Vec<Value> =
        used_families(s).into_iter().filter(|(f, _)| !db.has_family(f)).map(|(family, runs)| json!({"family": family, "runs": runs})).collect();
    Ok(json!({
        "complex": complex,
        "complexMissing": !missing.is_empty(),
        "missingChars": missing.iter().map(|c| format!("U+{:04X}", u32::from(*c))).collect::<Vec<_>>(),
        "missingFamilies": missing_families,
        "canAddFont": true,
        "canLoadSystemFonts": true,
    }))
}

/// Most characters a diagnostic reads (bounds fallback searches on a huge document).
const MAX_DIAGNOSTIC_TEXT: usize = 200_000;
/// Most distinct document families a diagnostic reports.
const MAX_DIAGNOSTIC_FAMILIES: usize = 500;

/// The whole document's text (body and every part), bounded.
fn document_text(s: &Session) -> String {
    let mut text = s.doc.plain_text(StoryRef::Body);
    for id in s.doc.parts.keys() {
        if text.chars().count() >= MAX_DIAGNOSTIC_TEXT {
            break;
        }
        text.push('\n');
        text.push_str(&s.doc.plain_text(StoryRef::Part(*id)));
    }
    text
}

/// The families the document asks for, and how many runs name each: resolved through the styles
/// (so a style's font counts), including East Asian and complex-script families. Bounded.
fn used_families(s: &Session) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let stories = std::iter::once(StoryRef::Body).chain(s.doc.parts.keys().map(|k| StoryRef::Part(*k)));
    'stories: for story in stories {
        for path in s.doc.para_paths(story) {
            let Some(p) = s.doc.para(story, &path) else { continue };
            let mut add = |rc: &wordcraft_doc::resolve::ResolvedChar| {
                for family in [Some(rc.font.as_str()), Some(rc.font_cs.as_str()), rc.font_ea.as_deref()].into_iter().flatten() {
                    let family = family.trim();
                    if !family.is_empty() {
                        *counts.entry(family.to_string()).or_default() += 1;
                    }
                }
            };
            add(&s.doc.styles.resolve_char(p.props.style.as_deref(), &p.mark));
            for run in &p.runs {
                add(&s.doc.styles.resolve_char(p.props.style.as_deref(), &run.props));
            }
            if counts.len() >= MAX_DIAGNOSTIC_FAMILIES {
                break 'stories;
            }
        }
    }
    counts.into_iter().collect()
}

fn load_system(s: &mut Session, _v: &Value) -> CmdResult {
    s.ui_requests.push(json!({"open": "loadSystemFonts"}));
    Ok(json!({"pending": true}))
}

fn font_json(f: &UserFont) -> Value {
    json!({
        "file": f.file,
        "families": f.families(),
        "faces": f.faces.iter().map(|(family, style)| json!({"family": family, "style": style})).collect::<Vec<_>>(),
        "path": f.path.as_ref().map(|p| p.to_string_lossy().to_string()),
    })
}

/// Read a font file named by the user, refusing one larger than the cap before reading it.
#[cfg(not(target_arch = "wasm32"))]
fn read_font_file(path: &str) -> Result<Vec<u8>, CmdError> {
    // Through `io`, which keeps automation inside its read root (#769).
    crate::io::read_file_capped(path, wordcraft_fonts::MAX_FONT_FILE_BYTES as u64, "font file").map_err(CmdError::Failed)
}

fn add(s: &mut Session, v: &Value) -> CmdResult {
    if p::str(v, "url").is_some() {
        return Err(CmdError::Failed(
            "adding a font from a link isn't available yet: download the font file and add it with {\"path\"} or {\"data\"}".into(),
        ));
    }
    let (name, bytes) = if let Some(d) = p::str(v, "data") {
        // Base64 is a third larger than the file: refuse before decoding an oversized one.
        if d.len() / 4 * 3 > wordcraft_fonts::MAX_FONT_FILE_BYTES + 3 {
            return Err(CmdError::Failed(format!("the font file is larger than {} MB", wordcraft_fonts::MAX_FONT_FILE_BYTES >> 20)));
        }
        let bytes = base64_decode(d).ok_or_else(|| CmdError::Params("bad base64 data".into()))?;
        (p::str(v, "name").unwrap_or("font").to_string(), bytes)
    } else if let Some(path) = p::str(v, "path") {
        #[cfg(not(target_arch = "wasm32"))]
        {
            (path.to_string(), read_font_file(path)?)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = path;
            return Err(CmdError::Failed("reading files by path isn't available on the web".into()));
        }
    } else {
        // From the ribbon: the UI shows a file picker and runs this again with the file.
        s.ui_requests.push(json!({"open": "addFont"}));
        return Ok(json!({"pending": true}));
    };
    let r = FontDb::global().install_font(&name, &bytes).map_err(|e| CmdError::Failed(e.to_string()))?;
    // Text already set in the family (shown in a fallback until now) takes the new font.
    s.relayout();
    let families = r.font.families().join(", ");
    s.status = match &r.warning {
        Some(w) => format!("Added {families} ({w})"),
        None => format!("Added {families}"),
    };
    let mut out = font_json(&r.font);
    if let Some(w) = r.warning {
        out["warning"] = json!(w);
    }
    Ok(out)
}

fn list(_: &mut Session, v: &Value) -> CmdResult {
    let db = FontDb::global();
    let mut out = json!({
        "fonts": db.user_fonts().iter().map(font_json).collect::<Vec<_>>(),
        "folder": db.user_font_dir().map(|p| p.to_string_lossy().to_string()),
    });
    if p::bool(v, "all") == Some(true) {
        out["families"] = json!(db.families());
    }
    Ok(out)
}

fn remove(s: &mut Session, v: &Value) -> CmdResult {
    let family = p::str(v, "family").filter(|f| !f.trim().is_empty()).ok_or_else(|| CmdError::Params("family is required".into()))?;
    let gone = FontDb::global().remove_user_font(family.trim()).map_err(|e| CmdError::Failed(e.to_string()))?;
    s.relayout();
    s.status = format!("Removed {}", family.trim());
    Ok(json!({"removed": gone.iter().map(font_json).collect::<Vec<_>>()}))
}
