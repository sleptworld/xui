use std::collections::BTreeMap;
use std::env;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=icons");
    println!("cargo:rerun-if-changed=build.rs");

    let icons_dir = Path::new("icons");
    let mut icons = BTreeMap::new();

    for entry in fs::read_dir(icons_dir).expect("failed to read Lucide icons directory") {
        let path = entry.expect("failed to read Lucide icon entry").path();
        if path.extension() != Some(OsStr::new("svg")) {
            continue;
        }

        let name = path
            .file_stem()
            .and_then(OsStr::to_str)
            .expect("Lucide icon filename must be valid UTF-8");
        let svg = fs::read_to_string(&path).expect("failed to read Lucide SVG");
        assert!(
            svg.contains("<svg"),
            "Lucide icon {} is not an SVG",
            path.display()
        );

        let ident = rust_ident(name);
        if let Some(previous) = icons.insert(ident.clone(), name.to_owned()) {
            panic!("Lucide icons {previous:?} and {name:?} both map to {ident:?}");
        }
    }

    assert!(!icons.is_empty(), "no Lucide SVG icons found");
    validate_exported_names(&icons);
    let generated = generate(&icons);
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is not set"));
    fs::write(out_dir.join("icons.rs"), generated).expect("failed to write generated icon index");
}

fn validate_exported_names(icons: &BTreeMap<String, String>) {
    let mut exported = BTreeMap::new();
    for (ident, name) in icons {
        for symbol in [ident.clone(), svg_ident(ident)] {
            if let Some(previous) = exported.insert(symbol.clone(), name) {
                panic!("Lucide icons {previous:?} and {name:?} both export {symbol:?}");
            }
        }
    }
}

fn generate(icons: &BTreeMap<String, String>) -> String {
    let mut output = String::new();
    writeln!(
        output,
        "/// Number of embedded icons in this Lucide release.\npub const ICON_COUNT: usize = {};",
        icons.len()
    )
    .unwrap();

    output.push_str(
        "/// Original kebab-case Lucide icon names.\npub const ICON_NAMES: &[&str] = &[\n",
    );
    for name in icons.values() {
        writeln!(output, "    {name:?},").unwrap();
    }
    output.push_str("];\n\n");

    output.push_str(
        "/// Returns an icon by its original kebab-case Lucide name.\n\
         ///\n\
         /// The SVG is parsed only on the first call for that icon.\n\
         pub fn get(name: &str) -> Option<xui::IconData> {\n    match name {\n",
    );
    for (ident, name) in icons {
        writeln!(output, "        {name:?} => Some(icons::{ident}()),").unwrap();
    }
    output.push_str("        _ => None,\n    }\n}\n\n");

    output.push_str(
        "/// Returns an embedded SVG by its original kebab-case Lucide name.\n\
         pub fn svg(name: &str) -> Option<&'static str> {\n    match name {\n",
    );
    for (ident, name) in icons {
        let svg_ident = svg_ident(ident);
        writeln!(output, "        {name:?} => Some(icons::{svg_ident}()),").unwrap();
    }
    output.push_str("        _ => None,\n    }\n}\n\n");

    output.push_str(
        "/// Type-safe accessors for every Lucide icon.\n\
         pub mod icons {\n    use std::sync::OnceLock;\n    use xui::IconData;\n\n",
    );
    for (ident, name) in icons {
        let svg_ident = svg_ident(ident);
        writeln!(
            output,
            "    #[doc = {:?}]",
            format!("Lucide `{name}` icon.")
        )
        .unwrap();
        writeln!(output, "    pub fn {ident}() -> IconData {{").unwrap();
        output.push_str("        static DATA: OnceLock<IconData> = OnceLock::new();\n");
        writeln!(
            output,
            "        DATA.get_or_init(|| IconData::from_svg({svg_ident}()).expect(\"embedded Lucide SVG must be valid\")).clone()"
        )
        .unwrap();
        output.push_str("    }\n\n");
        writeln!(
            output,
            "    #[doc = {:?}]",
            format!("Raw SVG for the Lucide `{name}` icon.")
        )
        .unwrap();
        writeln!(output, "    pub const fn {svg_ident}() -> &'static str {{").unwrap();
        writeln!(
            output,
            "        include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/icons/\", {name:?}, \".svg\"))"
        )
        .unwrap();
        output.push_str("    }\n\n");
    }
    output.push_str("}\n");
    output
}

fn svg_ident(icon_ident: &str) -> String {
    format!("{}_svg", icon_ident.strip_suffix('_').unwrap_or(icon_ident))
}

fn rust_ident(name: &str) -> String {
    let mut ident = String::with_capacity(name.len() + 5);
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            ident.push(ch.to_ascii_lowercase());
        } else {
            ident.push('_');
        }
    }

    if ident.starts_with(|ch: char| ch.is_ascii_digit()) {
        ident.insert_str(0, "icon_");
    }

    if RUST_KEYWORDS.contains(&ident.as_str()) {
        ident.push('_');
    }
    ident
}

const RUST_KEYWORDS: &[&str] = &[
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
    "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl",
    "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
    "return", "self", "static", "struct", "super", "trait", "true", "try", "type", "typeof",
    "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
];

#[cfg(test)]
mod tests {
    use super::{rust_ident, svg_ident};

    #[test]
    fn creates_valid_identifiers() {
        assert_eq!(rust_ident("search"), "search");
        assert_eq!(rust_ident("arrow-down"), "arrow_down");
        assert_eq!(rust_ident("3d-glasses"), "icon_3d_glasses");
        assert_eq!(rust_ident("move"), "move_");
        assert_eq!(svg_ident("search"), "search_svg");
        assert_eq!(svg_ident("move_"), "move_svg");
    }
}
