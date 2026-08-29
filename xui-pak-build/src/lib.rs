//! Build-time and command-line tooling for deterministic XPAK archives.
//!
//! Scans a source directory with `ignore`, applies glob `RuleConfig`s, encodes a
//! deterministic `.xpak` archive, and generates Rust asset-id constants. Used by
//! `xui-cli` (`cargo xui`) and `xui-pak-cli` (`xpak`), or directly from a
//! `build.rs` via `build`.
//!
//! Archives are reproducible: the same source and config produce byte-identical
//! `.xpak` files (entries are ordered by `AssetId`, not filesystem order). The
//! generated Rust mirrors the directory structure as nested `pub mod`s with
//! `pub const NAME: AssetId = AssetId::from_bytes([...]);` constants.

use std::{
    collections::{BTreeMap, HashMap},
    env, fs,
    io::Cursor,
    path::{Path, PathBuf},
};

use camino::{Utf8Path, Utf8PathBuf};
use globset::{Glob, GlobMatcher};
use ignore::WalkBuilder;
use serde::Deserialize;
use thiserror::Error;
use xui_pak::{
    AssetError, AssetId, Compression, HEADER_LEN,
    format::{PakIndex, PakIndexEntry, encode_header},
    normalize_asset_path,
};

#[derive(Debug, Error)]
pub enum BuildError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid config: {0}")]
    Config(#[from] toml::de::Error),
    #[error("invalid glob `{glob}`: {source}")]
    Glob {
        glob: String,
        source: globset::Error,
    },
    #[error(transparent)]
    Asset(#[from] AssetError),
    #[error("path is not valid UTF-8: {0}")]
    NonUtf8(PathBuf),
    #[error("invalid build configuration: {0}")]
    InvalidConfig(String),
    #[error("generated Rust name collision: {0}")]
    NameCollision(String),
    #[error("pak index encoding failed: {0}")]
    IndexEncode(#[from] postcard::Error),
    #[error("directory scan failed: {0}")]
    Walk(#[from] ignore::Error),
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct BuildConfig {
    pub package: PackageConfig,
    pub rules: Vec<RuleConfig>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct PackageConfig {
    pub source: Utf8PathBuf,
    pub output: String,
    pub generated: String,
    pub compression_level: i32,
    pub asset_id_path: String,
}

impl Default for PackageConfig {
    fn default() -> Self {
        Self {
            source: Utf8PathBuf::from("assets"),
            output: "assets.xpak".into(),
            generated: "xui_assets.rs".into(),
            compression_level: 3,
            asset_id_path: "xui_pak::AssetId".into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct RuleConfig {
    pub glob: String,
    pub compression: CompressionSetting,
    pub alignment: u32,
}

impl Default for RuleConfig {
    fn default() -> Self {
        Self {
            glob: "**/*".into(),
            compression: CompressionSetting::Auto,
            alignment: 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompressionSetting {
    #[default]
    Auto,
    None,
    Zstd,
}

#[derive(Clone, Debug)]
pub struct BuildOutput {
    pub pak_path: PathBuf,
    pub generated_path: PathBuf,
    pub asset_count: usize,
}

pub fn build(config_path: impl AsRef<Path>) -> Result<BuildOutput, BuildError> {
    let manifest_dir = env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| BuildError::InvalidConfig("CARGO_MANIFEST_DIR is not set".into()))?;
    let out_dir = env::var_os("OUT_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| BuildError::InvalidConfig("OUT_DIR is not set".into()))?;
    let config_path = resolve_from(&manifest_dir, config_path.as_ref());
    let config = load_config(&config_path)?;
    let source = resolve_from(
        config_path.parent().unwrap_or(&manifest_dir),
        config.package.source.as_std_path(),
    );
    let output = out_dir.join(&config.package.output);
    let generated = out_dir.join(&config.package.generated);
    let result = build_to(&config, &source, &output, &generated)?;

    println!("cargo:rerun-if-changed={}", config_path.display());
    println!("cargo:rerun-if-changed={}", source.display());
    Ok(result)
}

pub fn load_config(path: impl AsRef<Path>) -> Result<BuildConfig, BuildError> {
    Ok(toml::from_str(&fs::read_to_string(path)?)?)
}

pub fn build_from_config_to(
    config_path: impl AsRef<Path>,
    output_override: Option<&Path>,
) -> Result<BuildOutput, BuildError> {
    let config_path = config_path.as_ref();
    let config = load_config(config_path)?;
    let base = config_path.parent().unwrap_or_else(|| Path::new("."));
    let source = resolve_from(base, config.package.source.as_std_path());
    let output = output_override
        .map(PathBuf::from)
        .unwrap_or_else(|| base.join(&config.package.output));
    let generated = output
        .parent()
        .unwrap_or(base)
        .join(&config.package.generated);
    build_to(&config, &source, &output, &generated)
}

pub fn build_to(
    config: &BuildConfig,
    source: &Path,
    pak_path: &Path,
    generated_path: &Path,
) -> Result<BuildOutput, BuildError> {
    validate_file_name(&config.package.output, "package.output")?;
    validate_file_name(&config.package.generated, "package.generated")?;
    if !source.is_dir() {
        return Err(BuildError::InvalidConfig(format!(
            "source directory does not exist: {}",
            source.display()
        )));
    }
    let compiled_rules = compile_rules(&config.rules)?;
    let mut assets = scan_assets(source, &compiled_rules)?;
    assets.sort_by(|a, b| a.id.cmp(&b.id));
    for pair in assets.windows(2) {
        if pair[0].id == pair[1].id {
            return Err(BuildError::InvalidConfig(format!(
                "AssetId collision between `{}` and `{}`",
                pair[0].path, pair[1].path
            )));
        }
    }

    let pak = encode_pak(&assets, config.package.compression_level)?;
    validate_rust_path(&config.package.asset_id_path)?;
    let generated = generate_rust(&assets, &config.package.asset_id_path)?;
    if let Some(parent) = pak_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(parent) = generated_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(pak_path, pak)?;
    fs::write(generated_path, generated)?;
    Ok(BuildOutput {
        pak_path: pak_path.to_owned(),
        generated_path: generated_path.to_owned(),
        asset_count: assets.len(),
    })
}

#[derive(Clone)]
struct CompiledRule {
    matcher: GlobMatcher,
    compression: CompressionSetting,
    alignment: u32,
}

#[derive(Clone)]
struct InputAsset {
    id: AssetId,
    path: Utf8PathBuf,
    bytes: Vec<u8>,
    compression: Compression,
    alignment: u32,
}

fn compile_rules(rules: &[RuleConfig]) -> Result<Vec<CompiledRule>, BuildError> {
    rules
        .iter()
        .map(|rule| {
            if rule.alignment == 0 || !rule.alignment.is_power_of_two() || rule.alignment > 4096 {
                return Err(BuildError::InvalidConfig(format!(
                    "alignment must be a power of two between 1 and 4096 for `{}`",
                    rule.glob
                )));
            }
            let matcher = Glob::new(&rule.glob)
                .map_err(|source| BuildError::Glob {
                    glob: rule.glob.clone(),
                    source,
                })?
                .compile_matcher();
            Ok(CompiledRule {
                matcher,
                compression: rule.compression,
                alignment: rule.alignment,
            })
        })
        .collect()
}

fn scan_assets(source: &Path, rules: &[CompiledRule]) -> Result<Vec<InputAsset>, BuildError> {
    let source = source.canonicalize()?;
    let mut assets = Vec::new();
    let mut paths = HashMap::new();
    for item in WalkBuilder::new(&source)
        .follow_links(false)
        .hidden(false)
        .build()
    {
        let entry = item?;
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() || file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if path.symlink_metadata()?.file_type().is_symlink() {
            continue;
        }
        let relative = path.strip_prefix(&source).map_err(|_| {
            BuildError::InvalidConfig(format!("path escaped source root: {}", path.display()))
        })?;
        let relative = Utf8Path::from_path(relative)
            .ok_or_else(|| BuildError::NonUtf8(relative.to_owned()))?;
        let normalized = normalize_asset_path(relative.as_str())?;
        if paths.insert(normalized.clone(), ()).is_some() {
            return Err(BuildError::InvalidConfig(format!(
                "duplicate path `{normalized}`"
            )));
        }
        let mut compression = auto_compression(&normalized);
        let mut alignment = 1;
        for rule in rules {
            if rule.matcher.is_match(normalized.as_std_path()) {
                compression = match rule.compression {
                    CompressionSetting::Auto => auto_compression(&normalized),
                    CompressionSetting::None => Compression::None,
                    CompressionSetting::Zstd => Compression::Zstd,
                };
                alignment = rule.alignment;
            }
        }
        assets.push(InputAsset {
            id: AssetId::from_normalized_path(normalized.as_str()),
            path: normalized,
            bytes: fs::read(path)?,
            compression,
            alignment,
        });
    }
    Ok(assets)
}

fn auto_compression(path: &Utf8Path) -> Compression {
    const ALREADY_COMPRESSED: &[&str] = &[
        "7z", "avif", "br", "bz2", "gif", "gz", "jpeg", "jpg", "mp3", "mp4", "ogg", "otf", "png",
        "ttf", "webm", "webp", "woff", "woff2", "xz", "zip", "zst",
    ];
    match path.extension().map(str::to_ascii_lowercase) {
        Some(extension) if ALREADY_COMPRESSED.contains(&extension.as_str()) => Compression::None,
        _ => Compression::Zstd,
    }
}

fn encode_pak(assets: &[InputAsset], compression_level: i32) -> Result<Vec<u8>, BuildError> {
    let mut output = vec![0; HEADER_LEN];
    let mut entries = Vec::with_capacity(assets.len());
    for asset in assets {
        pad_to_alignment(&mut output, asset.alignment);
        let offset = output.len() as u64;
        let stored = match asset.compression {
            Compression::None => asset.bytes.clone(),
            Compression::Zstd => {
                zstd::stream::encode_all(Cursor::new(&asset.bytes), compression_level)?
            }
        };
        output.extend_from_slice(&stored);
        entries.push(PakIndexEntry {
            id: asset.id,
            path: asset.path.to_string(),
            content_hash: *blake3::hash(&asset.bytes).as_bytes(),
            offset,
            stored_len: stored.len() as u64,
            original_len: asset.bytes.len() as u64,
            compression: asset.compression,
            alignment: asset.alignment,
        });
    }
    let index_offset = output.len() as u64;
    let index = postcard::to_allocvec(&PakIndex { entries })?;
    let index_hash = *blake3::hash(&index).as_bytes();
    output.extend_from_slice(&index);
    let header = encode_header(index_offset, index.len() as u64, index_hash);
    output[..HEADER_LEN].copy_from_slice(&header);
    Ok(output)
}

fn pad_to_alignment(output: &mut Vec<u8>, alignment: u32) {
    let alignment = alignment as usize;
    let padding = (alignment - output.len() % alignment) % alignment;
    output.resize(output.len() + padding, 0);
}

#[derive(Default)]
struct ModuleNode {
    modules: BTreeMap<String, ModuleNode>,
    constants: BTreeMap<String, AssetId>,
}

fn generate_rust(assets: &[InputAsset], asset_id_path: &str) -> Result<String, BuildError> {
    let mut root = ModuleNode::default();
    for asset in assets {
        let components: Vec<_> = asset.path.as_str().split('/').collect();
        let (file, directories) = components.split_last().unwrap();
        let mut node = &mut root;
        for directory in directories {
            let name = rust_identifier(directory, false);
            if node.constants.contains_key(&name) {
                return Err(BuildError::NameCollision(asset.path.to_string()));
            }
            node = node.modules.entry(name).or_default();
        }
        let name = rust_identifier(file, true);
        if node.modules.contains_key(&name) || node.constants.insert(name, asset.id).is_some() {
            return Err(BuildError::NameCollision(asset.path.to_string()));
        }
    }
    let mut output = String::from("// @generated by xui-pak-build. Do not edit.\n");
    write_module(&root, 0, asset_id_path, &mut output);
    Ok(output)
}

fn write_module(node: &ModuleNode, depth: usize, asset_id_path: &str, output: &mut String) {
    let indent = "    ".repeat(depth);
    for (name, child) in &node.modules {
        output.push_str(&format!("{indent}pub mod {name} {{\n"));
        write_module(child, depth + 1, asset_id_path, output);
        output.push_str(&format!("{indent}}}\n"));
    }
    for (name, id) in &node.constants {
        output.push_str(&format!(
            "{indent}pub const {name}: {asset_id_path} = {asset_id_path}::from_bytes({:?});\n",
            id.as_bytes()
        ));
    }
}

fn rust_identifier(value: &str, uppercase: bool) -> String {
    let mut result = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            result.push(if uppercase {
                character.to_ascii_uppercase()
            } else {
                character.to_ascii_lowercase()
            });
        } else {
            result.push('_');
        }
    }
    if result.is_empty() || result.as_bytes()[0].is_ascii_digit() {
        result.insert(0, '_');
    }
    const KEYWORDS: &[&str] = &[
        "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn",
        "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
        "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe",
        "use", "where", "while", "async", "await", "dyn",
    ];
    if KEYWORDS.contains(&result.as_str()) {
        result.insert(0, '_');
    }
    result
}

fn resolve_from(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    }
}

fn validate_file_name(value: &str, field: &str) -> Result<(), BuildError> {
    if value.is_empty() || Path::new(value).file_name().and_then(|v| v.to_str()) != Some(value) {
        return Err(BuildError::InvalidConfig(format!(
            "{field} must be a file name"
        )));
    }
    Ok(())
}

fn validate_rust_path(value: &str) -> Result<(), BuildError> {
    if value.is_empty()
        || value.split("::").any(|segment| {
            segment.is_empty()
                || !segment
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
                || segment.as_bytes()[0].is_ascii_digit()
        })
    {
        return Err(BuildError::InvalidConfig(
            "package.asset_id_path must be a Rust path".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use xui_pak::{AssetBytes, AssetError, AssetSource, EmbeddedPak, PakOpenOptions, PakSource};

    #[test]
    fn build_is_deterministic_and_round_trips() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("assets");
        fs::create_dir_all(source.join("data")).unwrap();
        fs::write(source.join("data/empty.bin"), []).unwrap();
        fs::write(source.join("data/app.bin"), [0, 255, 1, 2, 3]).unwrap();
        let first = temp.path().join("first.xpak");
        let second = temp.path().join("second.xpak");
        let generated = temp.path().join("assets.rs");
        let config = BuildConfig::default();
        build_to(&config, &source, &first, &generated).unwrap();
        build_to(&config, &source, &second, &generated).unwrap();
        assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());

        let pak = PakSource::open(first).unwrap();
        let id = AssetId::from_path("data/app.bin").unwrap();
        let app = pak.load(id).unwrap().unwrap();
        assert_eq!(&*app.bytes, &[0, 255, 1, 2, 3]);
        assert!(matches!(app.bytes, AssetBytes::Owned(_)));
        pak.verify_all().unwrap();
        let generated = fs::read_to_string(generated).unwrap();
        assert!(generated.contains("pub mod data"));
        assert!(generated.contains("APP_BIN"));

        let limited = PakSource::open_with_options(
            &second,
            PakOpenOptions {
                max_decompressed_entry_size: 4,
                ..PakOpenOptions::default()
            },
        )
        .unwrap();
        assert!(matches!(
            limited.load(id).unwrap_err(),
            AssetError::LimitExceeded { .. }
        ));
    }

    #[test]
    fn content_changes_do_not_change_id() {
        let id = AssetId::from_path("data/app.bin").unwrap();
        assert_eq!(id, AssetId::from_path("data/app.bin").unwrap());
    }

    #[test]
    fn external_and_embedded_uncompressed_assets_are_zero_copy() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("assets");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("pixel.png"), b"not really a png").unwrap();
        let pak_path = temp.path().join("assets.xpak");
        build_to(
            &BuildConfig::default(),
            &source,
            &pak_path,
            &temp.path().join("assets.rs"),
        )
        .unwrap();
        let id = AssetId::from_path("pixel.png").unwrap();
        let external = PakSource::open(&pak_path)
            .unwrap()
            .load(id)
            .unwrap()
            .unwrap();
        assert!(matches!(external.bytes, AssetBytes::Mapped { .. }));

        let leaked = Box::leak(fs::read(&pak_path).unwrap().into_boxed_slice());
        let embedded = EmbeddedPak::new(leaked).unwrap().load(id).unwrap().unwrap();
        assert!(matches!(embedded.bytes, AssetBytes::Static { .. }));
    }

    #[test]
    fn corrupted_payload_fails_verification() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("assets");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("plain.png"), b"payload").unwrap();
        let pak_path = temp.path().join("assets.xpak");
        build_to(
            &BuildConfig::default(),
            &source,
            &pak_path,
            &temp.path().join("assets.rs"),
        )
        .unwrap();
        let mut bytes = fs::read(&pak_path).unwrap();
        bytes[HEADER_LEN] ^= 0xff;
        fs::write(&pak_path, bytes).unwrap();
        let error = PakSource::open(&pak_path)
            .unwrap()
            .verify_all()
            .unwrap_err();
        assert!(matches!(error, AssetError::HashMismatch(_)));
    }

    #[test]
    fn generated_name_collisions_are_errors() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("assets");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("a-b"), b"one").unwrap();
        fs::write(source.join("a_b"), b"two").unwrap();
        let error = build_to(
            &BuildConfig::default(),
            &source,
            &temp.path().join("assets.xpak"),
            &temp.path().join("assets.rs"),
        )
        .unwrap_err();
        assert!(matches!(error, BuildError::NameCollision(_)));
    }
}
