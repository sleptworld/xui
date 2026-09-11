//! Packs an XUI application's assets from its build script.
//!
//! ```toml
//! [build-dependencies]
//! xui-build = { path = "../xui-build" }
//! ```
//!
//! ```ignore
//! // build.rs
//! fn main() {
//!     xui_build::assets();
//! }
//! ```
//!
//! [`assets`] reads the package's optional `xui.toml`, packs its assets into
//! `OUT_DIR`, and hands the generated bootstrap module to the crate through
//! `cargo::rustc-env`, which is where `xui::include_assets!()` looks. Plain
//! `cargo`, rust-analyzer and every Cargo subcommand then build the
//! application with no wrapper, and Cargo reruns the script only when the
//! assets or their configuration change.
//!
//! `cargo xui` generates through the same code, so both routes produce the
//! same files.

use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

use camino::Utf8PathBuf;
use serde::Deserialize;
use thiserror::Error;
use xui_pak_build::{BuildConfig, PackageConfig, build_empty_to, build_to};
pub use xui_pak_build::{BuildOutput, RuleConfig};

pub const CONFIG_FILE: &str = "xui.toml";
/// Names the bootstrap module `xui::include_assets!()` includes.
pub const BOOTSTRAP_ENV: &str = "XUI_ASSETS_BOOTSTRAP";
/// Read by the application at startup: a directory to mount over its bundled
/// assets. Must match `xui::assets::bootstrap::DIR_ENV`.
pub const DIR_ENV: &str = "XUI_ASSETS_DIR";
/// Read by the application at startup: the external package to open. Must
/// match `xui::assets::bootstrap::PAK_ENV`.
pub const PAK_ENV: &str = "XUI_ASSETS_PAK";
const DEFAULT_SOURCE: &str = "assets";
const REFS_FILE: &str = "xui_asset_refs.rs";
const BOOTSTRAP_FILE: &str = "xui_assets_bootstrap.rs";

#[derive(Debug, Error)]
pub enum Error {
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid {}: {source}", path.display())]
    Config {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("{CONFIG_FILE} sets assets.source = {configured:?}, but {} is not a directory", path.display())]
    MissingSource { configured: String, path: PathBuf },
    #[error(transparent)]
    Build(#[from] xui_pak_build::BuildError),
    #[error("path is not valid UTF-8: {}", .0.display())]
    NonUtf8(PathBuf),
    #[error("{0} is not set; xui_build::assets() is meant to run from a build script")]
    NotABuildScript(&'static str),
}

/// The contents of `xui.toml`. Every key is optional.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub assets: AssetsConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AssetsConfig {
    /// `None` when not set, which is what lets a package without an assets
    /// directory build: an unset source may be missing, a set one may not.
    pub source: Option<Utf8PathBuf>,
    pub bundle: BundleMode,
    pub dev_directory: bool,
    pub output: String,
    pub compression_level: i32,
    pub rules: Vec<RuleConfig>,
}

impl Default for AssetsConfig {
    fn default() -> Self {
        Self {
            source: None,
            bundle: BundleMode::Embedded,
            dev_directory: true,
            output: "assets.xpak".into(),
            compression_level: 3,
            rules: Vec::new(),
        }
    }
}

impl AssetsConfig {
    pub fn source(&self) -> Utf8PathBuf {
        self.source
            .clone()
            .unwrap_or_else(|| Utf8PathBuf::from(DEFAULT_SOURCE))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BundleMode {
    #[default]
    Embedded,
    External,
}

/// Reads `xui.toml` from a package directory; `None` when there is none.
pub fn load_config(package_dir: &Path) -> Result<Option<Config>, Error> {
    let path = package_dir.join(CONFIG_FILE);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(Error::Io { path, source }),
    };
    toml::from_str(&text)
        .map(Some)
        .map_err(|source| Error::Config { path, source })
}

/// Where one package's generated files go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The assets directory, resolved against the package.
    pub source: PathBuf,
    pub pak: PathBuf,
    pub refs: PathBuf,
    pub bootstrap: PathBuf,
}

impl Layout {
    pub fn new(package_dir: &Path, generated_dir: &Path, config: &AssetsConfig) -> Self {
        Self {
            source: package_dir.join(config.source().as_std_path()),
            pak: generated_dir.join(&config.output),
            refs: generated_dir.join(REFS_FILE),
            bootstrap: generated_dir.join(BOOTSTRAP_FILE),
        }
    }
}

/// Where the application looks for a development directory and an external
/// package when the environment names neither.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fallbacks {
    /// Nowhere: `cargo xui` names both in the environment of everything it
    /// runs, so the bootstrap needs no paths of its own.
    None,
    /// In a debug build, the assets directory and the package this build
    /// produced -- so a plain `cargo run` works without a wrapper. Compiled
    /// out of release builds, which carry no paths from the build machine.
    Debug,
}

/// Packs a package's assets into `generated_dir` and writes the bootstrap
/// module next to them.
///
/// Files whose contents did not change are not rewritten, so nothing built
/// from them is invalidated.
pub fn generate(
    package_dir: &Path,
    generated_dir: &Path,
    config: &AssetsConfig,
    fallbacks: Fallbacks,
) -> Result<(Layout, BuildOutput), Error> {
    let layout = Layout::new(package_dir, generated_dir, config);
    let build_config = BuildConfig {
        package: PackageConfig {
            source: config.source(),
            output: config.output.clone(),
            generated: REFS_FILE.into(),
            compression_level: config.compression_level,
            asset_id_path: "xui::assets::AssetId".into(),
        },
        rules: config.rules.clone(),
    };
    let build = if layout.source.is_dir() {
        build_to(&build_config, &layout.source, &layout.pak, &layout.refs)?
    } else if let Some(configured) = &config.source {
        return Err(Error::MissingSource {
            configured: configured.to_string(),
            path: layout.source.clone(),
        });
    } else {
        // No assets yet, and nothing configured asking for any.
        build_empty_to(&build_config, &layout.pak, &layout.refs)?
    };
    let source = bootstrap_source(&layout, config, fallbacks)?;
    write_if_changed(&layout.bootstrap, source).map_err(|source| Error::Io {
        path: layout.bootstrap.clone(),
        source,
    })?;
    Ok((layout, build))
}

/// Packs the package's assets. Call it from `build.rs`.
///
/// A failure is reported to Cargo as a build error naming the problem --
/// `cargo::error` lines, not a panic -- and fails the build.
///
/// Cargo reruns the script when `xui.toml` or anything under the assets
/// directory changes. One case it cannot see: an `xui.toml` created after
/// the first build, in a package that already had its assets directory.
/// `cargo xui init` touches `build.rs` so that it does; after writing one by
/// hand, touch `build.rs` yourself.
pub fn assets() {
    if let Err(error) = try_assets() {
        for line in error.to_string().lines() {
            println!("cargo::error={line}");
        }
    }
}

fn try_assets() -> Result<(), Error> {
    let package_dir = env_path("CARGO_MANIFEST_DIR")?;
    let out_dir = env_path("OUT_DIR")?;
    let config_path = package_dir.join(CONFIG_FILE);
    // Watch the configuration before reading it, so fixing a broken one
    // reruns the script.
    if config_path.is_file() {
        println!("cargo::rerun-if-changed={}", config_path.display());
    }
    let config = load_config(&package_dir)?.unwrap_or_default().assets;
    let (layout, _) = generate(&package_dir, &out_dir, &config, Fallbacks::Debug)?;
    for path in watched(&config_path, &layout.source) {
        if path != config_path {
            println!("cargo::rerun-if-changed={}", path.display());
        }
    }
    println!(
        "cargo::rustc-env={BOOTSTRAP_ENV}={}",
        layout.bootstrap.display()
    );
    Ok(())
}

/// What the build script asks Cargo to watch.
///
/// Only paths that exist: Cargo treats a missing one as changed on every
/// build, and would rerun the script each time. Without an assets directory
/// nothing is named at all, which leaves Cargo's default -- rerun when any
/// file in the package changes -- so creating the directory is noticed.
fn watched(config: &Path, source: &Path) -> Vec<PathBuf> {
    if !source.is_dir() {
        return Vec::new();
    }
    [config, source]
        .into_iter()
        .filter(|path| path.exists())
        .map(Path::to_owned)
        .collect()
}

fn env_path(name: &'static str) -> Result<PathBuf, Error> {
    env::var_os(name)
        .map(PathBuf::from)
        .ok_or(Error::NotABuildScript(name))
}

/// The module `xui::include_assets!()` includes.
fn bootstrap_source(
    layout: &Layout,
    config: &AssetsConfig,
    fallbacks: Fallbacks,
) -> Result<String, Error> {
    let refs = rust_path_literal(&layout.refs)?;
    let debug = |path: Option<&Path>| -> Result<String, Error> {
        Ok(match path {
            Some(path) => rust_path_literal(path)?,
            None => String::new(),
        })
    };
    // A `let` per build kind rather than `cfg!`, so the path is not merely
    // unused in a release build but absent from it.
    let debug_binding = |name: &str, path: Option<&Path>| -> Result<String, Error> {
        Ok(match (fallbacks, path) {
            (Fallbacks::Debug, Some(_)) => format!(
                "        #[cfg(debug_assertions)]\n        let {name} = Some({});\n        #[cfg(not(debug_assertions))]\n        let {name} = None;\n",
                debug(path)?
            ),
            _ => format!("        let {name} = None;\n"),
        })
    };
    let dev_directory = (config.dev_directory && layout.source.is_dir()).then_some(&*layout.source);
    let mut body = debug_binding("dev_directory", dev_directory)?;
    body.push_str("        xui::assets::bootstrap::mount_overlay(&mut manager, dev_directory);\n");
    match config.bundle {
        BundleMode::Embedded => body.push_str(&format!(
            "        manager.mount(xui::assets::EmbeddedPak::new(include_bytes!({}))?);\n",
            rust_path_literal(&layout.pak)?
        )),
        BundleMode::External => {
            body.push_str(&debug_binding("fallback_pak", Some(&layout.pak))?);
            body.push_str(&format!(
                "        manager.mount(xui::assets::bootstrap::external_pak({:?}, fallback_pak)?);\n",
                config.output
            ));
        }
    }
    Ok(format!(
        "// @generated by xui-build. Do not edit.\n\
         pub mod xui_assets {{\n\
         \x20   pub mod refs {{ include!({refs}); }}\n\
         \x20   pub fn manager() -> Result<xui::assets::AssetManager, xui::assets::AssetError> {{\n\
         \x20       let mut manager = xui::assets::AssetManager::new();\n\
         {body}\
         \x20       Ok(manager)\n\
         \x20   }}\n\
         }}\n"
    ))
}

fn rust_path_literal(path: &Path) -> Result<String, Error> {
    let path = path
        .to_str()
        .ok_or_else(|| Error::NonUtf8(path.to_owned()))?;
    Ok(format!("{path:?}"))
}

/// Writes `contents` unless `path` already holds exactly that, and says
/// whether it wrote.
///
/// The bootstrap is `include!`d into the application crate and Cargo decides
/// staleness by mtime, so rewriting identical text would recompile the
/// application every time.
pub fn write_if_changed(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<bool> {
    let contents = contents.as_ref();
    if fs::read(path).is_ok_and(|existing| existing == contents) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xui_pak::{AssetId, PakSource};

    fn config(toml: &str) -> AssetsConfig {
        toml::from_str::<Config>(toml).unwrap().assets
    }

    #[test]
    fn a_package_without_assets_gets_an_empty_bundle() {
        let temp = tempfile::tempdir().unwrap();
        let out = temp.path().join("out");
        let (layout, build) = generate(
            temp.path(),
            &out,
            &AssetsConfig::default(),
            Fallbacks::Debug,
        )
        .unwrap();
        assert_eq!(build.asset_count, 0);
        assert_eq!(PakSource::open(&layout.pak).unwrap().entries().count(), 0);
        let bootstrap = fs::read_to_string(&layout.bootstrap).unwrap();
        // No assets directory, so nothing to mount even in a debug build.
        assert!(
            bootstrap.contains("let dev_directory = None;"),
            "{bootstrap}"
        );
    }

    #[test]
    fn assets_are_packed_and_named() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("assets/icons")).unwrap();
        fs::write(temp.path().join("assets/icons/search.svg"), b"<svg/>").unwrap();
        let (layout, build) = generate(
            temp.path(),
            &temp.path().join("out"),
            &AssetsConfig::default(),
            Fallbacks::None,
        )
        .unwrap();
        assert_eq!(build.asset_count, 1);
        assert_eq!(
            PakSource::open(&layout.pak)
                .unwrap()
                .entries()
                .next()
                .unwrap()
                .id,
            AssetId::from_path("icons/search.svg").unwrap()
        );
        assert!(
            fs::read_to_string(&layout.refs)
                .unwrap()
                .contains("pub const SEARCH_SVG")
        );
    }

    #[test]
    fn debug_fallbacks_are_compiled_out_of_release_builds() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("assets")).unwrap();
        let out = temp.path().join("out");
        let external = config("[assets]\nbundle = \"external\"\n");

        let (layout, _) = generate(temp.path(), &out, &external, Fallbacks::Debug).unwrap();
        let bootstrap = fs::read_to_string(&layout.bootstrap).unwrap();
        let source = format!("{:?}", layout.source.to_str().unwrap());
        let pak = format!("{:?}", layout.pak.to_str().unwrap());
        for (name, path) in [("dev_directory", &source), ("fallback_pak", &pak)] {
            let debug_only = format!(
                "#[cfg(debug_assertions)]\n        let {name} = Some({path});\n        #[cfg(not(debug_assertions))]\n        let {name} = None;"
            );
            assert!(bootstrap.contains(&debug_only), "{bootstrap}");
        }
        assert!(bootstrap.contains(r#"external_pak("assets.xpak", fallback_pak)"#));

        // For `cargo xui`, which names both in the environment instead.
        let (layout, _) = generate(temp.path(), &out, &external, Fallbacks::None).unwrap();
        let bootstrap = fs::read_to_string(&layout.bootstrap).unwrap();
        assert!(
            !bootstrap.contains(&source) && !bootstrap.contains(&pak),
            "{bootstrap}"
        );
    }

    #[test]
    fn only_existing_paths_are_watched() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join(CONFIG_FILE);
        let source = temp.path().join("assets");
        // Nothing yet: name nothing, so Cargo's default notices the first
        // assets directory being created.
        assert!(watched(&config_path, &source).is_empty());

        fs::create_dir(&source).unwrap();
        assert_eq!(watched(&config_path, &source), [source.clone()]);

        fs::write(&config_path, "").unwrap();
        assert_eq!(watched(&config_path, &source), [config_path, source]);
    }

    #[test]
    fn configuration_mistakes_name_the_problem() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join(CONFIG_FILE),
            "[assets]\ndev_directroy = false\n",
        )
        .unwrap();
        let error = load_config(temp.path()).unwrap_err().to_string();
        assert!(
            error.contains(CONFIG_FILE) && error.contains("dev_directroy"),
            "{error}"
        );

        let error = generate(
            temp.path(),
            &temp.path().join("out"),
            &config("[assets]\nsource = \"art\"\n"),
            Fallbacks::Debug,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("assets.source = \"art\""), "{error}");

        assert!(load_config(&temp.path().join("absent")).unwrap().is_none());
    }
}
