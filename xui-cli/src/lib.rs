//! `cargo xui` — builds XUI assets from `xui.toml` before invoking Cargo.
//!
//! Reads `xui.toml` next to the application's `Cargo.toml`, packs its assets
//! into a deterministic `.xpak`, generates a bootstrap module exposing
//! `xui_assets::refs` and `xui_assets::manager()`, sets the
//! `XUI_ASSETS_BOOTSTRAP` env var, and then invokes Cargo so
//! `xui::include_assets!()` can find the bootstrap module.
//!
//! Commands: `build`, `run`, `check`, `test`, `clippy`, and `assets {pack,list,verify}`.
//! Cargo arguments (including `--release`, `--target`, `--manifest-path`, and a
//! trailing `-- <args>` separator) are forwarded verbatim.

use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, ExitCode, ExitStatus},
};

use camino::Utf8PathBuf;
use clap::{Args, Parser, Subcommand};
use serde::Deserialize;
use thiserror::Error;
use xui_pak::PakSource;
use xui_pak_build::{build_to, BuildConfig, BuildOutput, PackageConfig, RuleConfig};

#[derive(Debug, Error)]
pub enum CliError {
    #[error(transparent)]
    Args(#[from] clap::Error),
    #[error("{0}")]
    Usage(&'static str),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid xui.toml: {0}")]
    Config(#[from] toml::de::Error),
    #[error(transparent)]
    Build(#[from] xui_pak_build::BuildError),
    #[error(transparent)]
    Asset(#[from] xui_pak::AssetError),
    #[error("cargo metadata failed")]
    MetadataFailed,
    #[error("invalid cargo metadata: {0}")]
    Metadata(#[from] serde_json::Error),
    #[error("cargo command exited with {0}")]
    CargoFailed(ExitStatus),
    #[error("path is not valid UTF-8: {0}")]
    NonUtf8(PathBuf),
}

impl CliError {
    pub fn print(&self) {
        match self {
            Self::Args(error) => {
                let _ = error.print();
            }
            error => eprintln!("cargo-xui: {error}"),
        }
    }

    pub fn exit_code(&self) -> ExitCode {
        match self {
            Self::Args(error) => u8::try_from(error.exit_code())
                .map(ExitCode::from)
                .unwrap_or(ExitCode::FAILURE),
            _ => ExitCode::FAILURE,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "cargo-xui",
    bin_name = "cargo xui",
    version,
    about = "Build XUI assets before invoking Cargo.",
    arg_required_else_help = true,
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: CliCommand,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Build the package with generated XUI assets.
    Build(CargoArgs),
    /// Run a binary with generated XUI assets.
    Run(CargoArgs),
    /// Check the package with generated XUI assets.
    Check(CargoArgs),
    /// Test the package with generated XUI assets.
    Test(CargoArgs),
    /// Run Clippy with generated XUI assets.
    Clippy(CargoArgs),
    /// Build, inspect, or validate the generated asset package.
    Assets(AssetsCommand),
}

#[derive(Debug, Args)]
struct AssetsCommand {
    #[command(subcommand)]
    action: AssetAction,
}

#[derive(Debug, Subcommand)]
enum AssetAction {
    /// Build the configured asset package.
    Pack(CargoArgs),
    /// List entries in the configured asset package.
    List(CargoArgs),
    /// Verify entries in the configured asset package.
    Verify(CargoArgs),
}

#[derive(Debug, Args)]
#[command(disable_help_flag = true)]
struct CargoArgs {
    #[arg(
        value_name = "CARGO_OPTIONS",
        num_args = 0..,
        allow_hyphen_values = true,
        trailing_var_arg = true
    )]
    args: Vec<OsString>,
}

#[derive(Clone, Copy, Debug)]
enum AssetActionKind {
    Pack,
    List,
    Verify,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct XuiConfig {
    assets: AssetsConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
struct AssetsConfig {
    source: Utf8PathBuf,
    bundle: BundleMode,
    dev_directory: bool,
    output: String,
    compression_level: i32,
    rules: Vec<RuleConfig>,
}

impl Default for AssetsConfig {
    fn default() -> Self {
        Self {
            source: Utf8PathBuf::from("assets"),
            bundle: BundleMode::Embedded,
            dev_directory: true,
            output: "assets.xpak".into(),
            compression_level: 3,
            rules: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum BundleMode {
    #[default]
    Embedded,
    External,
}

#[derive(Deserialize)]
struct CargoMetadata {
    target_directory: PathBuf,
}

struct PreparedAssets {
    build: BuildOutput,
    bootstrap: PathBuf,
    external_destination: Option<PathBuf>,
}

pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), CliError> {
    let cli = Cli::try_parse_from(normalize_args(args))?;

    match cli.command {
        CliCommand::Build(args) => run_cargo("build", args.args),
        CliCommand::Run(args) => run_cargo("run", args.args),
        CliCommand::Check(args) => run_cargo("check", args.args),
        CliCommand::Test(args) => run_cargo("test", args.args),
        CliCommand::Clippy(args) => run_cargo("clippy", args.args),
        CliCommand::Assets(command) => run_assets(command),
    }
}

fn normalize_args(args: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    let mut args: Vec<OsString> = args.into_iter().collect();
    if args.is_empty() {
        args.push(OsString::from("cargo-xui"));
    }
    // Cargo passes the subcommand name to `cargo-xui`; direct invocation does not.
    if args.get(1).is_some_and(|arg| arg == "xui") {
        args.remove(1);
    }
    args
}

fn run_cargo(subcommand: &str, cargo_args: Vec<OsString>) -> Result<(), CliError> {
    let manifest = manifest_path(&cargo_args)?;
    let profile = profile_from_args(&cargo_args);
    let target = option_value(&cargo_args, "--target");
    let target_dir = option_value(&cargo_args, "--target-dir");
    let prepared = prepare_project(
        &manifest,
        &profile,
        target,
        target_dir,
        matches!(subcommand, "run" | "test"),
    )?;
    if let Some(destination) = &prepared.external_destination {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&prepared.build.pak_path, destination)?;
    }

    let mut cargo = ProcessCommand::new(cargo_executable());
    cargo.arg(subcommand).args(&cargo_args);
    if option_value(&cargo_args, "--manifest-path").is_none() {
        cargo.arg("--manifest-path").arg(&manifest);
    }
    cargo.env("XUI_ASSETS_BOOTSTRAP", &prepared.bootstrap);
    let status = cargo.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(CliError::CargoFailed(status))
    }
}

fn run_assets(command: AssetsCommand) -> Result<(), CliError> {
    let (action, args) = match command.action {
        AssetAction::Pack(args) => (AssetActionKind::Pack, args.args),
        AssetAction::List(args) => (AssetActionKind::List, args.args),
        AssetAction::Verify(args) => (AssetActionKind::Verify, args.args),
    };
    let manifest = manifest_path(&args)?;
    let profile = profile_from_args(&args);
    let target = option_value(&args, "--target");
    let target_dir = option_value(&args, "--target-dir");
    let prepared = prepare_project(&manifest, &profile, target, target_dir, false)?;
    match action {
        AssetActionKind::Pack => println!(
            "packed {} assets into {}",
            prepared.build.asset_count,
            prepared.build.pak_path.display()
        ),
        AssetActionKind::List => {
            let pak = PakSource::open(&prepared.build.pak_path)?;
            for entry in pak.entries() {
                println!(
                    "{}\t{}\t{:?}\t{}",
                    entry.original_len, entry.stored_len, entry.compression, entry.path
                );
            }
        }
        AssetActionKind::Verify => {
            let pak = PakSource::open(&prepared.build.pak_path)?;
            pak.verify_all()?;
            println!("verified {}", prepared.build.pak_path.display());
        }
    }
    Ok(())
}

fn prepare_project(
    manifest: &Path,
    profile: &str,
    target: Option<&OsStr>,
    target_dir_override: Option<&OsStr>,
    development_run: bool,
) -> Result<PreparedAssets, CliError> {
    let manifest = manifest.canonicalize()?;
    let project_dir = manifest
        .parent()
        .ok_or(CliError::Usage("manifest path has no parent"))?;
    let config_path = project_dir.join("xui.toml");
    let config: XuiConfig = toml::from_str(&fs::read_to_string(&config_path)?)?;
    let metadata = cargo_metadata(&manifest)?;
    let target_directory = match target_dir_override {
        Some(path) if Path::new(path).is_absolute() => PathBuf::from(path),
        Some(path) => env::current_dir()?.join(path),
        None => metadata.target_directory,
    };
    let generated_dir = target_directory.join("xui").join(profile);
    fs::create_dir_all(&generated_dir)?;

    let source = project_dir.join(config.assets.source.as_std_path());
    let pak_path = generated_dir.join(&config.assets.output);
    let refs_path = generated_dir.join("xui_asset_refs.rs");
    let build_config = BuildConfig {
        package: PackageConfig {
            source: config.assets.source.clone(),
            output: config.assets.output.clone(),
            generated: "xui_asset_refs.rs".into(),
            compression_level: config.assets.compression_level,
            asset_id_path: "xui::assets::AssetId".into(),
        },
        rules: config.assets.rules.clone(),
    };
    let build = build_to(&build_config, &source, &pak_path, &refs_path)?;
    let bootstrap = generated_dir.join("xui_assets_bootstrap.rs");
    fs::write(
        &bootstrap,
        bootstrap_source(
            &refs_path,
            &pak_path,
            &source,
            config.assets.bundle,
            development_run && config.assets.dev_directory,
            &config.assets.output,
        )?,
    )?;

    let external_destination = match config.assets.bundle {
        BundleMode::Embedded => None,
        BundleMode::External => {
            let mut directory = target_directory;
            if let Some(target) = target {
                directory.push(target);
            }
            directory.push(cargo_profile_directory(profile));
            Some(directory.join(&config.assets.output))
        }
    };
    Ok(PreparedAssets {
        build,
        bootstrap,
        external_destination,
    })
}

fn bootstrap_source(
    refs_path: &Path,
    pak_path: &Path,
    source: &Path,
    bundle: BundleMode,
    dev_directory: bool,
    output_name: &str,
) -> Result<String, CliError> {
    let refs_path = rust_path_literal(refs_path)?;
    let pak_path = rust_path_literal(pak_path)?;
    let source = rust_path_literal(source)?;
    let output_name = format!("{output_name:?}");
    let mut mounts = String::new();
    if dev_directory {
        mounts.push_str(&format!(
            "        manager.mount(xui::assets::DirectorySource::new({source})?);\n"
        ));
    }
    match bundle {
        BundleMode::Embedded => mounts.push_str(&format!(
            "        manager.mount(xui::assets::EmbeddedPak::new(include_bytes!({pak_path}))?);\n"
        )),
        BundleMode::External => mounts.push_str(&format!(
            "        let executable = std::env::current_exe()?;\n        let directory = executable.parent().ok_or_else(|| xui::assets::AssetError::InvalidPath(executable.display().to_string()))?;\n        let packaged = directory.join({output_name});\n        let pak = if packaged.is_file() {{ packaged }} else {{ std::path::PathBuf::from({pak_path}) }};\n        manager.mount(xui::assets::PakSource::open(pak)?);\n"
        )),
    }
    Ok(format!(
        "// @generated by cargo-xui. Do not edit.\n\
         pub mod xui_assets {{\n\
         \x20   pub mod refs {{ include!({refs_path}); }}\n\
         \x20   pub fn manager() -> Result<xui::assets::AssetManager, xui::assets::AssetError> {{\n\
         \x20       let mut manager = xui::assets::AssetManager::new();\n\
         {mounts}\
         \x20       Ok(manager)\n\
         \x20   }}\n\
         }}\n"
    ))
}

fn cargo_metadata(manifest: &Path) -> Result<CargoMetadata, CliError> {
    let output = ProcessCommand::new(cargo_executable())
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(manifest)
        .output()?;
    if !output.status.success() {
        return Err(CliError::MetadataFailed);
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn manifest_path(args: &[OsString]) -> Result<PathBuf, CliError> {
    if let Some(value) = option_value(args, "--manifest-path") {
        Ok(PathBuf::from(value))
    } else {
        Ok(env::current_dir()?.join("Cargo.toml"))
    }
}

fn profile_from_args(args: &[OsString]) -> String {
    if let Some(profile) = option_value(args, "--profile") {
        profile.to_string_lossy().into_owned()
    } else if args
        .iter()
        .take_while(|arg| *arg != "--")
        .any(|arg| arg == "--release")
    {
        "release".into()
    } else {
        "debug".into()
    }
}

fn cargo_profile_directory(profile: &str) -> &str {
    if profile == "dev" {
        "debug"
    } else {
        profile
    }
}

fn option_value<'a>(args: &'a [OsString], name: &str) -> Option<&'a OsStr> {
    for (index, arg) in args.iter().enumerate() {
        if arg == "--" {
            break;
        }
        if arg == name {
            return args.get(index + 1).map(OsString::as_os_str);
        }
        if let Some(value) = arg
            .to_str()
            .and_then(|arg| arg.strip_prefix(&format!("{name}=")))
        {
            return Some(OsStr::new(value));
        }
    }
    None
}

fn rust_path_literal(path: &Path) -> Result<String, CliError> {
    let path = path
        .to_str()
        .ok_or_else(|| CliError::NonUtf8(path.to_owned()))?;
    Ok(format!("{path:?}"))
}

fn cargo_executable() -> OsString {
    env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use xui_pak::AssetId;
    use xui_pak_build::CompressionSetting;

    #[test]
    fn config_generates_embedded_bootstrap_and_refs() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("assets/data")).unwrap();
        fs::write(temp.path().join("assets/data/app.bin"), b"data").unwrap();
        let refs = temp.path().join("refs.rs");
        let pak = temp.path().join("assets.xpak");
        let config = BuildConfig {
            package: PackageConfig {
                source: Utf8PathBuf::from("assets"),
                output: "assets.xpak".into(),
                generated: "refs.rs".into(),
                compression_level: 3,
                asset_id_path: "xui::assets::AssetId".into(),
            },
            rules: vec![RuleConfig {
                glob: "**/*".into(),
                compression: CompressionSetting::None,
                alignment: 1,
            }],
        };
        build_to(&config, &temp.path().join("assets"), &pak, &refs).unwrap();
        let generated = fs::read_to_string(&refs).unwrap();
        assert!(generated.contains("xui::assets::AssetId"));
        assert_eq!(
            AssetId::from_path("data/app.bin").unwrap(),
            PakSource::open(&pak).unwrap().entries().next().unwrap().id
        );
        let bootstrap = bootstrap_source(
            &refs,
            &pak,
            &temp.path().join("assets"),
            BundleMode::Embedded,
            true,
            "assets.xpak",
        )
        .unwrap();
        assert!(bootstrap.contains("DirectorySource"));
        assert!(bootstrap.contains("EmbeddedPak"));
        assert!(bootstrap.contains("pub mod xui_assets"));
    }

    #[test]
    fn cargo_options_do_not_inspect_application_arguments() {
        let args = vec![
            OsString::from("--target=wasm32-unknown-unknown"),
            OsString::from("--"),
            OsString::from("--release"),
        ];
        assert_eq!(
            option_value(&args, "--target"),
            Some(OsStr::new("wasm32-unknown-unknown"))
        );
        assert_eq!(profile_from_args(&args), "debug");
    }

    #[test]
    fn clap_parses_cargo_subcommand_invocation() {
        let cli = Cli::try_parse_from(normalize_args([
            OsString::from("cargo-xui"),
            OsString::from("xui"),
            OsString::from("build"),
            OsString::from("--help"),
            OsString::from("--target=wasm32-unknown-unknown"),
            OsString::from("--"),
            OsString::from("--release"),
        ]))
        .unwrap();

        let CliCommand::Build(CargoArgs { args }) = cli.command else {
            panic!("expected build command");
        };
        assert_eq!(
            args,
            vec![
                OsString::from("--help"),
                OsString::from("--target=wasm32-unknown-unknown"),
                OsString::from("--"),
                OsString::from("--release"),
            ]
        );
    }

    #[test]
    fn clap_parses_asset_actions_with_cargo_options() {
        let cli = Cli::try_parse_from(normalize_args([
            OsString::from("cargo-xui"),
            OsString::from("assets"),
            OsString::from("verify"),
            OsString::from("--manifest-path"),
            OsString::from("app/Cargo.toml"),
        ]))
        .unwrap();

        let CliCommand::Assets(AssetsCommand {
            action: AssetAction::Verify(CargoArgs { args }),
        }) = cli.command
        else {
            panic!("expected assets verify command");
        };
        assert_eq!(
            args,
            vec![
                OsString::from("--manifest-path"),
                OsString::from("app/Cargo.toml"),
            ]
        );
    }
}
