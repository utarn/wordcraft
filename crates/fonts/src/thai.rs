//! One ordered, per-platform substitution policy for Thai (#15).
//!
//! Word substitutes a missing Thai document font from a platform list, and the shaper falls back
//! to Thai faces when a Latin face lacks Thai. Both used to keep their own lists, which disagreed
//! about Leelawadee, Thonburi, Sathu and the serif families. This module holds the single policy:
//! each request has a [`Role`], and every role has one ordered list per [`Platform`]. The font
//! database (Word font substitution) and [`crate::script`] (script fallback) both read it, so a
//! document's Thai font resolves the same way wherever the fallback starts.
//!
//! Order is Word's platform order, never metric closeness: within each coverage pass the first
//! installed family in the list wins, and only faces of the *same* family are compared by weight
//! and slant (the font database does that when it picks a style). TH Sarabun New always comes
//! before TH SarabunPSK for general text; serif requests use serif-only lists, so a serif request
//! never lands on a loopless sans. Tahoma, Microsoft Sans Serif and Arial Unicode MS are
//! Thai-capable *Latin* families: their Latin text stays in the family's normal Latin substitute,
//! and only their Thai text uses the Thai list.

/// A platform whose font substitution order the policy follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOs,
    Linux,
}

/// The platform this build runs on. Web (wasm) uses the Linux/BSD list, the open Thai faces.
pub const fn current_platform() -> Platform {
    #[cfg(windows)]
    {
        Platform::Windows
    }
    #[cfg(target_os = "macos")]
    {
        Platform::MacOs
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Platform::Linux
    }
}

/// What kind of family a document asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// A Thai sans or general document family, and the general Thai script fallback.
    /// TH Sarabun New, then TH SarabunPSK, then the platform's Thai faces.
    Sans,
    /// A Thai serif document family. Serif-first: never resolves to a loopless sans design.
    Serif,
    /// A Latin family that also carries Thai (Tahoma, Microsoft Sans Serif, Arial Unicode MS).
    /// Its Latin text keeps the normal Latin substitute; Thai text uses the general Thai faces.
    CapableLatin,
}

/// The installed families a resolution can choose from. A test can pass its own inventory; the
/// font database implements it for the real system.
pub trait Inventory {
    /// Is `family` available?
    fn has_family(&self, family: &str) -> bool;
    /// Does `family`'s Regular face map `c`? Inventories that do not track coverage say yes.
    fn covers(&self, _family: &str, _c: char) -> bool {
        true
    }
}

impl Inventory for crate::FontDb {
    fn has_family(&self, family: &str) -> bool {
        crate::FontDb::has_family(self, family)
    }
    fn covers(&self, family: &str, c: char) -> bool {
        let face = self.face(family, "Regular");
        face.named(family) && face.covers(c)
    }
}

impl Inventory for std::collections::HashSet<&str> {
    fn has_family(&self, family: &str) -> bool {
        self.iter().any(|f| f.eq_ignore_ascii_case(family))
    }
}

// General (sans) lists: TH Sarabun New and TH SarabunPSK first, then the platform's own Thai
// faces, then the Thai-capable Latin families. Every listed face carries Thai letters.

const SANS_WINDOWS: &[&str] = &[
    "TH Sarabun New",
    "TH SarabunPSK",
    "Cordia New",
    "Leelawadee UI",
    "Leelawadee",
    "Tahoma",
    "Noto Sans Thai",
    "Browallia New",
    "Angsana New",
    "Microsoft Sans Serif",
    "Arial Unicode MS",
];

const SANS_MACOS: &[&str] = &[
    "TH Sarabun New",
    "TH SarabunPSK",
    "Thonburi",
    "Ayuthaya",
    "Sathu",
    "Krungthep",
    "Silom",
    "Noto Sans Thai",
    "Tahoma",
    "Microsoft Sans Serif",
    "Arial Unicode MS",
];

const SANS_LINUX: &[&str] = &[
    "TH Sarabun New",
    "TH SarabunPSK",
    "Noto Sans Thai",
    "Laksaman",
    "Garuda",
    "Waree",
    "Tlwg Typo",
    "Loma",
    "Umpush",
    "Tahoma",
    "Noto Serif Thai",
    "Microsoft Sans Serif",
    "Arial Unicode MS",
];

// Serif lists: serif Thai designs only. No TH Sarabun (sans), Cordia, Leelawadee, Tahoma or
// Noto Sans Thai: a serif request must not silently become loopless sans. Browallia is a serif
// design, like Angsana.

const SERIF_WINDOWS: &[&str] = &[
    "Angsana New",
    "AngsanaUPC",
    "Browallia New",
    "BrowalliaUPC",
    "EucrosiaUPC",
    "JasmineUPC",
    "KodchiangUPC",
    "DilleniaUPC",
    "Noto Serif Thai",
    "Kinnari",
    "Norasi",
];

const SERIF_MACOS: &[&str] = &["Sathu", "Thonburi", "Ayuthaya", "Noto Serif Thai", "Kinnari", "Norasi"];

const SERIF_LINUX: &[&str] = &["Noto Serif Thai", "Norasi", "Kinnari"];

/// The ordered substitution list for `role` on `platform`.
pub fn candidates(role: Role, platform: Platform) -> &'static [&'static str] {
    match role {
        // A Thai-capable Latin family keeps its own Latin substitute; its Thai characters use
        // the same general Thai list as a Thai sans document family.
        Role::Sans | Role::CapableLatin => match platform {
            Platform::Windows => SANS_WINDOWS,
            Platform::MacOs => SANS_MACOS,
            Platform::Linux => SANS_LINUX,
        },
        Role::Serif => match platform {
            Platform::Windows => SERIF_WINDOWS,
            Platform::MacOs => SERIF_MACOS,
            Platform::Linux => SERIF_LINUX,
        },
    }
}

/// The role of a document family name. Thai serif names are checked before the general Thai
/// names because the families overlap (Angsana New, Browallia New, the UPC serif faces).
pub fn role(family: &str) -> Option<Role> {
    let f = family.trim().to_ascii_lowercase();
    if is_serif(&f) {
        Some(Role::Serif)
    } else if is_capable_latin(&f) {
        Some(Role::CapableLatin)
    } else if is_sans(&f) {
        Some(Role::Sans)
    } else {
        None
    }
}

/// A Thai document family (sans or serif). A missing one is drawn in one Thai face for the
/// whole run, never in a Latin substitute (#1549).
pub fn is_document_family(family: &str) -> bool {
    matches!(role(family), Some(Role::Sans | Role::Serif))
}

/// Is `family` a Latin family that Word recognises as Thai-capable? Its Latin text keeps the
/// family's ordinary Latin substitute; only Thai characters use a Thai face.
pub fn is_capable_latin_family(family: &str) -> bool {
    role(family) == Some(Role::CapableLatin)
}

/// The first installed family in `role`'s list, in Word's platform order. `None` when none of
/// the listed faces is installed: the caller then degrades to the coverage search.
pub fn first_installed(role: Role, platform: Platform, inventory: &impl Inventory) -> Option<&'static str> {
    candidates(role, platform).iter().copied().find(|f| inventory.has_family(f))
}

/// The installed family for `role` that best draws a whole run, in Word's platform order.
///
/// `cover_sets` are tried in order (for a mixed Thai/Latin run, first Thai letters *and* Latin
/// letters/digits, then Thai letters alone). Within each pass the *first* installed family that
/// covers the set wins, so coverage never reorders the policy's ranks; only faces of the same
/// family are ordered by metric closeness when the font database picks a style. The first
/// installed family is the last resort, so an installed Thai face with poor coverage still wins
/// over a Latin one.
pub fn substitute_for(role: Role, platform: Platform, inventory: &impl Inventory, cover_sets: &[&[char]]) -> Option<&'static str> {
    let installed: Vec<&'static str> = candidates(role, platform).iter().copied().filter(|f| inventory.has_family(f)).collect();
    for set in cover_sets {
        if let Some(found) = installed.iter().copied().find(|f| set.iter().all(|c| inventory.covers(f, *c))) {
            return Some(found);
        }
    }
    installed.first().copied()
}

/// Thai sans document families: Word's Thai families (Cordia New, Leelawadee, Leelawadee UI),
/// the Thai national fonts (TH Sarabun New, TH SarabunPSK and the other "TH …" fonts), the
/// Mac's, the Thai Linux Working Group's and Noto's. Matched by name only.
fn is_sans(f: &str) -> bool {
    const NAMES: &[&str] = &[
        "cordia new",
        "cordiaupc",
        "leelawadee",
        "leelawadee ui",
        "leelawadee ui semilight",
        "th sarabun new",
        "th sarabunpsk",
        "th sarabun psk",
        "th sarabun",
        "thonburi",
        "ayuthaya",
        "sathu",
        "krungthep",
        "silom",
        "sukhumvit set",
        "garuda",
        "laksaman",
        "loma",
        "purisa",
        "sawasdee",
        "umpush",
        "waree",
        "tlwg typo",
        "tlwg typist",
        "tlwg mono",
    ];
    NAMES.contains(&f) || f.ends_with("upc") || f.starts_with("th ") || f.contains("thai")
}

/// Serif Thai document families (Angsana New, Browallia New, EucrosiaUPC, Norasi, Kinnari…).
fn is_serif(f: &str) -> bool {
    const NAMES: &[&str] = &[
        "angsana new",
        "angsanaupc",
        "browallia new",
        "browalliaupc",
        "eucrosiaupc",
        "jasmineupc",
        "kodchiangupc",
        "dilleniaupc",
        "norasi",
        "kinnari",
        // The Mac's serif Thai designs.
        "sathu",
        "thonburi",
        "ayuthaya",
    ];
    NAMES.contains(&f) || (f.contains("thai") && f.contains("serif"))
}

/// Latin families Word treats as Thai-capable: they carry Thai glyphs of their own, but they are
/// Latin designs, not Thai document families.
fn is_capable_latin(f: &str) -> bool {
    const NAMES: &[&str] = &["tahoma", "microsoft sans serif", "arial unicode ms"];
    NAMES.contains(&f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const PLATFORMS: [Platform; 3] = [Platform::Windows, Platform::MacOs, Platform::Linux];

    fn inventory(families: &[&'static str]) -> HashSet<&'static str> {
        families.iter().copied().collect()
    }

    #[test]
    fn every_platform_starts_general_text_with_sarabun_new_then_psk() {
        for platform in PLATFORMS {
            let list = candidates(Role::Sans, platform);
            assert_eq!(list.first().copied(), Some("TH Sarabun New"), "{platform:?}");
            assert_eq!(list.get(1).copied(), Some("TH SarabunPSK"), "{platform:?}");
            // Serif requests never include the sans Sarabun designs.
            assert!(!candidates(Role::Serif, platform).iter().any(|f| f.contains("Sarabun")), "{platform:?}");
        }
    }

    #[test]
    fn sarabun_new_wins_over_psk_and_psk_wins_when_new_is_absent() {
        for platform in PLATFORMS {
            // Installed order must not matter: the policy order does.
            let both = inventory(&["TH SarabunPSK", "TH Sarabun New"]);
            assert_eq!(first_installed(Role::Sans, platform, &both), Some("TH Sarabun New"), "{platform:?}");
            let only_psk = inventory(&["TH SarabunPSK"]);
            assert_eq!(first_installed(Role::Sans, platform, &only_psk), Some("TH SarabunPSK"), "{platform:?}");
            // A serif request never resolves to either: they are sans designs.
            assert_eq!(first_installed(Role::Serif, platform, &both), None, "{platform:?}");
        }
    }

    #[test]
    fn every_platform_has_a_serif_only_serif_list() {
        // The loopless sans designs that used to creep into the serif list.
        const SANS_ONLY: &[&str] = &[
            "TH Sarabun New",
            "TH SarabunPSK",
            "Cordia New",
            "CordiaUPC",
            "Leelawadee",
            "Leelawadee UI",
            "Tahoma",
            "Microsoft Sans Serif",
            "Arial Unicode MS",
            "Noto Sans Thai",
            "Laksaman",
            "Garuda",
            "Waree",
            "Krungthep",
        ];
        for platform in PLATFORMS {
            let serif = candidates(Role::Serif, platform);
            assert!(!serif.is_empty(), "{platform:?}");
            for name in SANS_ONLY {
                assert!(!serif.contains(name), "{platform:?} serif list holds sans {name}");
            }
            // Serif designs the platform list offers are usable, and the open serif face is there.
            assert!(serif.contains(&"Noto Serif Thai"), "{platform:?}");
        }
    }

    #[test]
    fn a_serif_request_resolves_to_a_serif_face_on_every_platform() {
        for platform in PLATFORMS {
            let serif = candidates(Role::Serif, platform);
            let installed: HashSet<&str> = serif.iter().copied().filter(|f| !f.contains("Noto")).collect();
            assert!(!installed.is_empty(), "{platform:?}");
            let found = first_installed(Role::Serif, platform, &installed).expect("a listed serif face");
            assert!(serif.contains(&found), "{platform:?}");
            // The chosen face is classified as serif, never as a general/sans design.
            assert_eq!(role(found), Some(Role::Serif), "{platform:?} {found}");
        }
        // Angsana New is serif; Browallia is a serif design too.
        assert_eq!(role("Angsana New"), Some(Role::Serif));
        assert_eq!(role("Browallia New"), Some(Role::Serif));
        assert_eq!(role("BrowalliaUPC"), Some(Role::Serif));
    }

    #[test]
    fn thai_capable_latin_families_are_neither_unknown_nor_pure_thai() {
        for name in ["Tahoma", "tahoma", "Microsoft Sans Serif", "Arial Unicode MS", "ARIAL UNICODE MS"] {
            assert_eq!(role(name), Some(Role::CapableLatin), "{name}");
            assert!(is_capable_latin_family(name), "{name}");
            assert!(!is_document_family(name), "{name}");
        }
        // Their Thai characters use one general Thai face.
        assert_eq!(candidates(Role::CapableLatin, Platform::Windows), candidates(Role::Sans, Platform::Windows));
        // Pure Thai fonts and unknown Latin families stay as they were.
        assert_eq!(role("TH Sarabun New"), Some(Role::Sans));
        assert!(!is_capable_latin_family("TH Sarabun New"));
        assert_eq!(role("Calibri"), None);
        assert!(!is_document_family("Tahoma"));
    }

    #[test]
    fn word_order_outranks_metric_closeness_and_coverage_never_reorders_it() {
        // A later, "closer" face must not jump an earlier policy rank.
        let installed = inventory(&["Noto Sans Thai", "TH SarabunPSK"]);
        assert_eq!(substitute_for(Role::Sans, Platform::Linux, &installed, &[]), Some("TH SarabunPSK"));
        // Coverage is a second pass: an earlier installed face that covers nothing still wins
        // over a later face that covers the run (the same candidate order).
        struct Nocover(HashSet<&'static str>);
        impl Inventory for Nocover {
            fn has_family(&self, family: &str) -> bool {
                self.0.iter().any(|f| f.eq_ignore_ascii_case(family))
            }
            fn covers(&self, family: &str, _c: char) -> bool {
                family.eq_ignore_ascii_case("TH SarabunPSK")
            }
        }
        let inv = Nocover(inventory(&["TH Sarabun New", "TH SarabunPSK"]));
        assert_eq!(substitute_for(Role::Sans, Platform::Linux, &inv, &[&['\u{0E19}', 'A']]), Some("TH SarabunPSK"));
        // No suitable face: None, so the caller can degrade to the coverage search.
        let empty = inventory(&[]);
        assert_eq!(first_installed(Role::Sans, Platform::Linux, &empty), None);
        assert_eq!(substitute_for(Role::Serif, Platform::Linux, &empty, &[]), None);
    }

    #[test]
    fn non_thai_families_have_no_thai_role() {
        for name in ["Calibri", "Arial", "Times New Roman", "David", "B Nazanin", "Noto Sans Hebrew", "Definitely Not A Font"] {
            assert_eq!(role(name), None, "{name}");
            assert!(!is_document_family(name), "{name}");
            assert!(!is_capable_latin_family(name), "{name}");
        }
    }
}
