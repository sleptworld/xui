//! The complete Lucide icon set, embedded as SVG and exposed as XUI [`IconData`](xui::IconData).
//!
//! Each accessor parses its SVG once, on first use, and cheaply clones the cached icon data:
//!
//! ```
//! let search = lucide_rs::icons::search();
//! let by_name = lucide_rs::get("search").unwrap();
//! assert_eq!(search, by_name);
//! ```

include!(concat!(env!("OUT_DIR"), "/icons.rs"));

#[cfg(test)]
mod tests {
    use super::{ICON_COUNT, ICON_NAMES, get, svg};

    #[test]
    fn generated_index_matches_embedded_icons() {
        assert_eq!(ICON_NAMES.len(), ICON_COUNT);
        assert_eq!(ICON_COUNT, 1_756);
        assert!(ICON_NAMES.windows(2).all(|names| names[0] < names[1]));
    }

    #[test]
    fn every_embedded_svg_parses_as_xui_icon_data() {
        for &name in ICON_NAMES {
            assert!(svg(name).is_some(), "missing embedded SVG for {name}");
            let icon = get(name).unwrap_or_else(|| panic!("failed to find {name}"));
            assert!(icon.view_box().width > 0.0, "invalid width for {name}");
            assert!(icon.view_box().height > 0.0, "invalid height for {name}");
            assert!(!icon.layers().is_empty(), "no visible layers for {name}");
        }
    }

    #[test]
    fn unknown_name_is_absent() {
        assert!(get("not-a-lucide-icon").is_none());
        assert!(svg("not-a-lucide-icon").is_none());
    }
}
