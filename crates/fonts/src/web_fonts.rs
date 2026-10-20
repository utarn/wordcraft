// The faces a web (wasm32) build embeds from craft-fonts, where there is no system font to fall
// back to. `build.rs` filters craft-fonts' manifest with [`embedded_in_web`]; the fonts crate uses
// the same filter in its tests, so the web set cannot quietly lose a face.
//
// Noto Sans Georgian Regular is the Georgian interface face. Noto Sans Thai Regular is the last-resort
// Thai face for documents: the file is a variable font whose weight axis also gives Bold, so one
// manifest row covers both weights.

/// Is this craft-fonts face embedded in a web build?
pub fn embedded_in_web(family: &str, style: &str) -> bool {
    (family == "Noto Sans Georgian" && style == "Regular") || (family == "Noto Sans Thai" && style == "Regular")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_web_set_holds_the_ui_and_thai_faces_only() {
        assert!(embedded_in_web("Noto Sans Georgian", "Regular"));
        assert!(embedded_in_web("Noto Sans Thai", "Regular"));
        // The Thai face is a variable font: one file covers every weight, so Bold is not a row.
        assert!(!embedded_in_web("Noto Sans Thai", "Bold"));
        assert!(!embedded_in_web("Noto Sans Georgian", "Bold"));
        for family in ["BIZ UDPGothic", "BIZ UDMincho", "Shippori Mincho", "Noto Sans CJK SC", "Noto Sans Arabic"] {
            assert!(!embedded_in_web(family, "Regular"), "{family} stays out of the web build");
        }
    }
}
