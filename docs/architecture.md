# WordCraft architecture

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-20 · **Change:** minor (the layer-1 `thai` crate segments Thai with the ICU dictionary compiled to an embedded FST and supplies layout's break opportunities and proofing's words; `fonts` carries the ordered Thai substitution policy, theme/`w:cs` resolution and AAT shaping, `layout` carries Thai dictionary wrapping and Thai Distributed, and `proof` checks Thai (#14, #16, #17, #18, #19, #20); previously minor: `proof` is per language: English and Polish; previously major: first committed version; `plan/architecture.md` is local-only) · **Target:** Microsoft Word (Microsoft 365) for Mac 16.113.4

How WordCraft is built today (origin/main `40c5d51`). The longer design notes in
`plan/architecture.md` are gitignored and local to the original author's machine; this file is the
committed, current summary. Rules for working in the code are in [`AGENTS.md`](../AGENTS.md).

## Layers

~89,000 lines of Rust, 17 crates, 3 apps and `xtask`. Layering is enforced by `cargo xtask layers`:
a crate may only depend on crates in lower layers, and nothing below L6 depends on egui, eframe,
winit or rfd.

| Layer | Crate | Lines | Job |
|---|---|---|---|
| L0 | `geom` | 0.2k | Units (points, twips, EMU, inches, cm), rectangles, measurement parsing |
| L1 | `doc` | 10.0k | Document model: stories, paragraphs, runs, tables, sections, styles, numbering, fields, notes, comments, revisions, equations (OMML-shaped math tree, linear/LaTeX input); edit primitives; bidi |
| L1 | `fonts` | 2.4k | Font database (bundled OFL + system fonts, expanded XDG/Flatpak/Snap/Nix/X11/BSD discovery), metrics, outlines, shaping with HarfRust (GSUB, GPOS/GDEF or AAT `morx`), Word font substitution (`word.rs`: Carlito ↔ Calibri, Caladea ↔ Cambria, Liberation ↔ Arial/Times/Courier) and one ordered per-platform Thai policy (TH Sarabun New → TH SarabunPSK → platform list; serif-only lists), per-run/per-script Thai fallback for `w:cs`/`w:cstheme`/`<a:font script="Thai">`, OpenType MATH |
| L1 | `proof` | 3.9k | Proofing per language (`ProofLang` from a run's `w:lang`, ten proofed languages): English (public-domain word list), Polish, Portuguese, Italian, Croatian, Turkish, Russian, Czech, Indonesian and Thai spelling and suggestions over the shared affix engine (Croatian, Czech, Indonesian, Italian, Polish, Portuguese, Russian and Turkish from installed Hunspell dictionaries; Thai from a runtime `th_TH` dictionary with TIS-620 decoding, else the bundled CC0 PyThaiNLP list), rule-based grammar for English, Polish, Portuguese, Italian, Croatian and Turkish, hyphenation for English, Polish, Portuguese, Italian, Russian and Turkish; no Thai grammar or hyphenation |
| L1 | `thai` | 0.4k | Thai word segmentation and line breaking: the Unicode-licensed ICU Thai word list compiled by `build.rs` to an embedded FST (no filesystem, so web works). `segments` gives byte-offset words to `proof`; `breaks` gives guarded line-break opportunities (no end on a leading vowel, no start on a combining mark, paiyannoi/maiyamok attached) to `layout`. Only dependency: `fst` |
| L1 | `metafile` | 3.2k | Windows WMF and EMF metafile records parsed into a neutral vector model (paths and bitmaps) for layout and rendering; `kurbo` |
| L2 | `layout` | 9.7k | Shaping runs, line breaking (UAX #14 plus `thai::breaks` dictionary opportunities guarded for Thai typography), tabs, lists, tables (row splitting, floating tables), pagination, headers/footers, notes, floating objects and wrap, math layout, fields, hit testing, Thai Distributed line distribution; produces a display list |
| L2 | `docx` | 8.4k | OOXML WordprocessingML reader and writer (Transitional and Strict namespaces, `.docx/.docm/.dotx/.dotm`), OMML, custom properties, VBA parts preserved |
| L2 | `docbin` | 4.4k | Word 97-2003 `.doc` reader ([MS-DOC]: FIB, piece table, FKPs, SPRMs, stylesheet, lists, tables, notes, pictures) over a CFB container |
| L2 | `formats` | 10.6k | TXT, Markdown, HTML, RTF, ODT and LaTeX import/export through a shared intermediate model |
| L3 | `render` | 0.6k | vello_cpu rasteriser: pages and thumbnails from the display list; PNG/JPEG |
| L3 | `pdf` | 1.6k | krilla PDF export: font subsets, selectable text, links, outline, tagging |
| L4 | `engine` | 15.2k | `Session` (document, selection, undo, layout cache, view state), **command registry** (422 `CommandSpec`s in `cmd/*.rs`), Word feature catalog (`catalog.rs`), samples, file I/O dispatch (`io.rs`), Read Aloud sentence model |
| L4 | `zotero` | 1.7k | Zotero's word-processor integration protocol answered against a `Session` |
| L4 | `control-key` | 0.1k | Per-window control-port key files shared by the app and the MCP bridge |
| L5 | `mcp` | 1.0k | MCP server: every command as a tool; headless or bridged to the running app |
| L6 | `ui-egui` | 13.1k | Word-style front end: ribbon, keytips, mini-toolbar, Backstage, canvas, rulers, panes, dialogs, Equation tab, i18n catalogs, themes, control channel, headless `ui_shot` example |
| apps | `wordcraft` | 1.6k | Desktop app (eframe + wgpu; logging, window geometry, file associations) |
| apps | `wordcraft-cli` | 0.4k | `convert`, `text`, `inspect`, `run`, `commands`, `parity`, `mcp`, `zotero` |
| apps | `wordcraft-web` | 0.3k | Browser build (eframe web runner, WebGPU with WebGL2 fallback) |

## Data flow

1. **Open:** `engine::io` picks a reader by extension (`docx`, `docbin`, `formats`, JSON) and
   produces a `doc::Document`.
2. **Edit:** every user-visible action is a command id run through `Session::run` (ribbon,
   shortcuts, command search, CLI, control channel and MCP all dispatch by id). `Session::run`
   snapshots for undo (blocks are `Arc`-shared, so snapshots are cheap) and catches an escaped
   panic, restoring the document (never-crash standard).
3. **Layout:** `layout` turns the document into pages of positioned glyphs, boxes and images
   (display list), incrementally: an edit relayouts only what changed (1.4 ms on a 188-page
   document, measured 2026-10-06).
4. **Draw:** the UI paints pages through `render` (CPU raster, cached per page, pixel-aligned) on
   an egui/wgpu canvas; PDF export walks the same display list through `pdf`.
5. **Save:** `engine::io` picks a writer by extension.

## Testing and gates

- 821 `#[test]` functions and 10 property-test blocks: model, layout, DOCX round-trip and hostile
  input, `.doc`, formats, typing parity (`engine/src/tests_typing.rs`), bidi, i18n catalogs, UI
  interaction (egui_kittest), MCP acceptance; `hostile_params_never_panic` runs every command with
  junk params.
- `cargo xtask ci`: fmt, clippy `-D warnings`, tests, asset attribution, layering, wasm build.
- `cargo xtask parity` regenerates [`parity-checklist.md`](parity-checklist.md); a test keeps the
  catalog percentage above a floor that only rises.
- GitHub Actions on PRs: packaging lint, and FreeBSD / Windows ARM64 when build files change;
  release builds on pushes to `release`.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-20 | minor | Added the layer-1 `thai` crate (`fst` + embedded ICU word list; segmentation for proofing, guarded breaks for layout) and the layer-1 `metafile` crate, which the table had omitted; recorded the Thai work in `fonts` (ordered per-platform substitution, run-scoped per-script fallback, `w:cs`/theme, AAT), `layout` (dictionary wrapping, Thai Distributed) and `proof` (ten proofed languages) |
| 2026-10-10 | minor | `proof`: proofing languages (English, Polish), affix engine, Polish patterns; layout and engine route each run by its language |
| 2026-10-10 | major | First committed architecture doc, from the crates' sources and `AGENTS.md` |
