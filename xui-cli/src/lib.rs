//! `cargo xui` — XUI's companion to Cargo.
//!
//! An application normally packs its assets from its own build script (see
//! `xui-build`), and plain `cargo` builds it. `cargo xui` adds what a build
//! script cannot do:
//!
//! - `init` sets a package up: `xui.toml`, an assets directory, `build.rs`
//!   and the `xui-build` build-dependency. It is safe to rerun, and migrates
//!   a package that predates build scripts.
//! - Forwarded Cargo commands (`build`, `run`, `test`, and any other) get the
//!   runtime side: the development directory mounted, and an external asset
//!   package copied beside the executable Cargo builds.
//! - `assets {pack,list,verify}` inspects and validates packages.
//!
//! For a package without a build script, forwarded commands also generate
//! the bootstrap module and point `XUI_ASSETS_BOOTSTRAP` at it, as before.
//!
//! The package is found the way Cargo does -- from the current directory
//! upwards, or through `--manifest-path` and `-p`.

use std::{
    borrow::Cow,
    env,
    ffi::{OsStr, OsString},
    fmt, fs,
    io::{self, IsTerminal},
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, ExitCode, ExitStatus},
    time::SystemTime,
};

use clap::{Args, Parser, Subcommand};
use serde::Deserialize;
use thiserror::Error;
use xui_build::{
    AssetsConfig, BOOTSTRAP_ENV, BuildOutput, BundleMode, CONFIG_FILE, DIR_ENV, Fallbacks, Layout,
    PAK_ENV,
};
use xui_pak::{Compression, PakSource};

/// The name the build-dependency goes by, as `cargo metadata` reports it.
const BUILD_CRATE: &str = "xui-build";

#[derive(Debug, Error)]
pub enum CliError {
    #[error(transparent)]
    Args(#[from] clap::Error),
    #[error("{0}")]
    Usage(String),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("{}: {source}", path.display())]
    File {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Assets(#[from] xui_build::Error),
    #[error("{}: {source}", path.display())]
    Pak {
        path: PathBuf,
        #[source]
        source: xui_pak::AssetError,
    },
    #[error("{0}")]
    Metadata(String),
    #[error("could not read `cargo metadata` output: {0}")]
    MetadataJson(#[from] serde_json::Error),
    #[error("could not run cargo: {0}")]
    Spawn(#[source] io::Error),
    /// Cargo has already said why, so this one prints nothing.
    #[error("cargo exited with {0}")]
    CargoFailed(ExitStatus),
}

impl CliError {
    pub fn print(&self) {
        match self {
            Self::Args(error) => {
                let _ = error.print();
            }
            Self::CargoFailed(_) => {}
            error => {
                let label = paint(color_stderr(None), "1;31", "error:");
                eprintln!("{label} {error}");
            }
        }
    }

    /// Cargo's own exit code when Cargo failed, so a script or CI job sees
    /// the same code it would from plain `cargo` -- and, under `cargo xui
    /// run`, the application's.
    pub fn exit_code(&self) -> ExitCode {
        let code = match self {
            Self::Args(error) => error.exit_code(),
            Self::CargoFailed(status) => status.code().unwrap_or(1),
            _ => 1,
        };
        u8::try_from(code)
            .map(ExitCode::from)
            .unwrap_or(ExitCode::FAILURE)
    }
}

fn file_error(path: &Path) -> impl FnOnce(io::Error) -> CliError + '_ {
    move |source| CliError::File {
        path: path.to_owned(),
        source,
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "cargo-xui",
    bin_name = "cargo xui",
    version,
    about = "XUI's companion to Cargo: set up assets, run with them mounted, inspect packages.",
    after_help = AFTER_HELP,
    arg_required_else_help = true,
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: CliCommand,
}

const AFTER_HELP: &str = "\
Every other Cargo subcommand (doc, bench, nextest, ...) is forwarded the same
way, with its arguments passed through unchanged.

The package is found the way Cargo finds it: from the current directory
upwards, or through --manifest-path and -p. At a workspace root, the member
set up for XUI assets is used.";

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Build the package, with its assets prepared.
    Build(CargoArgs),
    /// Run a binary, with the development directory mounted.
    Run(CargoArgs),
    /// Check the package, with its assets prepared.
    Check(CargoArgs),
    /// Test the package, with its assets prepared.
    Test(CargoArgs),
    /// Run Clippy, with the package's assets prepared.
    Clippy(CargoArgs),
    /// Set a package up for XUI assets: xui.toml, assets/, build.rs and the
    /// xui-build build-dependency. Safe to rerun.
    Init(ProjectArgs),
    /// Build, inspect, or validate an asset package.
    Assets(AssetsCommand),
    #[command(external_subcommand)]
    External(Vec<OsString>),
}

#[derive(Debug, Args)]
struct AssetsCommand {
    #[command(subcommand)]
    action: AssetAction,
}

#[derive(Debug, Subcommand)]
enum AssetAction {
    /// Build the asset package.
    Pack(PackArgs),
    /// List the entries of an asset package.
    List(InspectArgs),
    /// Check every entry of an asset package against its content hash.
    Verify(InspectArgs),
}

/// Everything up to `--`, and the program's own arguments after it, handed
/// to Cargo as they are.
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

#[derive(Debug, Clone, Default, Args)]
struct ProjectArgs {
    /// Path to the package's Cargo.toml.
    #[arg(long, value_name = "PATH")]
    manifest_path: Option<PathBuf>,
    /// The package to use, when run from a workspace.
    #[arg(short, long, value_name = "SPEC")]
    package: Option<String>,
}

#[derive(Debug, Clone, Default, Args)]
struct PackArgs {
    #[command(flatten)]
    project: ProjectArgs,
    /// Use the release profile's output directory.
    #[arg(short, long)]
    release: bool,
    /// Use the named profile's output directory.
    #[arg(long, value_name = "NAME", conflicts_with = "release")]
    profile: Option<String>,
    /// Directory for generated files, as Cargo's --target-dir.
    #[arg(long, value_name = "DIR")]
    target_dir: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct InspectArgs {
    /// The package file to inspect. Defaults to the one this project builds.
    #[arg(value_name = "PAK")]
    pak: Option<PathBuf>,
    #[command(flatten)]
    pack: PackArgs,
}

impl PackArgs {
    fn options(&self) -> CargoOptions {
        CargoOptions {
            manifest_path: self.project.manifest_path.clone(),
            packages: self.project.package.iter().cloned().collect(),
            release: self.release,
            profile: self.profile.clone(),
            target_dir: self.target_dir.clone(),
            // The command prints its own result.
            quiet: true,
            ..CargoOptions::default()
        }
    }
}

pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), CliError> {
    let cli = Cli::try_parse_from(normalize_args(args))?;

    match cli.command {
        CliCommand::Build(args) => run_cargo(OsStr::new("build"), args.args),
        CliCommand::Run(args) => run_cargo(OsStr::new("run"), args.args),
        CliCommand::Check(args) => run_cargo(OsStr::new("check"), args.args),
        CliCommand::Test(args) => run_cargo(OsStr::new("test"), args.args),
        CliCommand::Clippy(args) => run_cargo(OsStr::new("clippy"), args.args),
        CliCommand::External(mut args) => {
            let subcommand = args.remove(0);
            run_cargo(&subcommand, args)
        }
        CliCommand::Init(args) => run_init(&args),
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

fn run_cargo(subcommand: &OsStr, args: Vec<OsString>) -> Result<(), CliError> {
    let options = CargoOptions::parse(&args);
    let mut cargo = ProcessCommand::new(cargo_executable());
    cargo.arg(subcommand).args(&args);
    // Help needs nothing built, and has to work in a project that would fail
    // to pack.
    if !options.help
        && let Resolution::Package(project) = resolve_project(
            options.manifest_path.as_deref(),
            &options.packages,
            options.workspace,
        )?
    {
        let prepared = prepare(&project, &options)?;
        if let Some(bootstrap) = &prepared.bootstrap {
            cargo.env(BOOTSTRAP_ENV, bootstrap);
        }
        if let Some(directory) = &prepared.dev_directory {
            cargo.env(DIR_ENV, directory);
        }
        if let Some(external) = &prepared.external {
            copy_if_changed(&prepared.layout.pak, &external.destination)?;
            cargo.env(PAK_ENV, &prepared.layout.pak);
        }
    }
    exec(cargo)
}

/// Hands over to Cargo. On Unix this replaces the process, so signals, the
/// terminal and the exit code are Cargo's own, exactly as with plain `cargo`.
fn exec(mut cargo: ProcessCommand) -> Result<(), CliError> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Only returns if Cargo could not be started.
        Err(CliError::Spawn(cargo.exec()))
    }
    #[cfg(not(unix))]
    {
        let status = cargo.status().map_err(CliError::Spawn)?;
        if status.success() {
            Ok(())
        } else {
            Err(CliError::CargoFailed(status))
        }
    }
}

/// Sets a package up, creating whatever part of the setup is missing and
/// leaving the rest alone -- so it also migrates a package that predates
/// build scripts.
fn run_init(args: &ProjectArgs) -> Result<(), CliError> {
    let project = resolve_project(
        args.manifest_path.as_deref(),
        args.package.as_slice(),
        false,
    )?
    .into_package()?;
    println!("Setting up XUI assets in {}", project.dir.display());
    let report = |done: bool, what: &str| {
        let verb = if done { "created" } else { "kept" };
        println!("  {verb:<8} {what}");
    };

    let config = project.dir.join(CONFIG_FILE);
    let created_config = !config.exists();
    if created_config {
        fs::write(&config, INIT_CONFIG).map_err(file_error(&config))?;
    }
    report(created_config, CONFIG_FILE);

    let assets_config = xui_build::load_config(&project.dir)?
        .unwrap_or_default()
        .assets;
    let assets = project.dir.join(assets_config.source().as_std_path());
    let created_assets = !assets.exists();
    if created_assets {
        fs::create_dir_all(&assets).map_err(file_error(&assets))?;
    }
    report(created_assets, &format!("{}/", assets_config.source()));

    let build_script = project.dir.join("build.rs");
    let mut notes = Vec::new();
    if !build_script.exists() {
        fs::write(&build_script, INIT_BUILD_SCRIPT).map_err(file_error(&build_script))?;
        report(true, "build.rs");
    } else if fs::read_to_string(&build_script)
        .map_err(file_error(&build_script))?
        .contains("xui_build::assets")
    {
        // Cargo does not watch an `xui.toml` that did not exist when the
        // script last ran; a newer build.rs makes it run again and see it.
        if created_config {
            touch(&build_script)?;
        }
        report(false, "build.rs");
    } else {
        notes.push("build.rs already exists: add `xui_build::assets();` to its `main`.".to_owned());
    }

    if project.uses_build_script {
        report(false, &format!("{BUILD_CRATE} in [build-dependencies]"));
    } else {
        match &project.xui_build_path {
            Some(path) => {
                add_build_dependency(&project.manifest, path)?;
                report(true, &format!("{BUILD_CRATE} in [build-dependencies]"));
            }
            None => notes.push(format!(
                "add {BUILD_CRATE} to [build-dependencies], from wherever the `xui` \
                 dependency comes from."
            )),
        }
    }

    for note in notes {
        println!("\nnote: {note}");
    }
    println!(
        "\nFiles under {}/ are named by constants in `xui_assets::refs`, in a crate that \
         uses `#[xui::main]`. Plain `cargo` builds it from here on.",
        assets_config.source()
    );
    Ok(())
}

fn touch(path: &Path) -> Result<(), CliError> {
    fs::File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(SystemTime::now()))
        .map_err(file_error(path))
}

/// `cargo add --build`, so the manifest keeps its formatting and gets a path
/// relative to itself, the way Cargo writes one.
fn add_build_dependency(manifest: &Path, crate_path: &Path) -> Result<(), CliError> {
    let output = ProcessCommand::new(cargo_executable())
        .args(["add", "--quiet", "--build", BUILD_CRATE, "--path"])
        .arg(crate_path)
        .arg("--manifest-path")
        .arg(manifest)
        .output()
        .map_err(CliError::Spawn)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(CliError::Usage(format!(
            "`cargo add --build {BUILD_CRATE}` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

const INIT_BUILD_SCRIPT: &str = "fn main() {\n    xui_build::assets();\n}\n";

/// Every key with its default, commented out, so the file documents itself
/// without pinning anything.
const INIT_CONFIG: &str = r#"# XUI asset settings. Every key is optional; the values shown are the defaults.

[assets]
# Directory whose files are packed, relative to this file.
# source = "assets"

# "embedded" builds the package into the executable; "external" ships it
# beside the executable, named `output`.
# bundle = "embedded"
# output = "assets.xpak"

# Mount `source` over the package in debug builds and under `cargo xui`, so
# edits show without a rebuild.
# dev_directory = true

# compression_level = 3

# Per-file overrides; the last matching rule wins.
# [[assets.rules]]
# glob = "**/*.bin"
# compression = "none"   # "auto", "none" or "zstd"
# alignment = 16
"#;

fn run_assets(command: AssetsCommand) -> Result<(), CliError> {
    match command.action {
        AssetAction::Pack(args) => {
            let (layout, build) = pack(&args)?;
            println!(
                "packed {} ({}) into {}",
                count(build.asset_count, "asset"),
                human_size(build.source_bytes),
                layout.pak.display()
            );
        }
        AssetAction::List(args) => {
            let path = match args.pak {
                Some(path) => path,
                None => pack(&args.pack)?.0.pak,
            };
            print!("{}", listing(&open_pak(&path)?));
        }
        AssetAction::Verify(args) => {
            // The package on disk, not a fresh one: checking a file this
            // command had just written would prove nothing.
            let path = match args.pak {
                Some(path) => path,
                None => {
                    let options = args.pack.options();
                    let project = resolve_project(
                        options.manifest_path.as_deref(),
                        &options.packages,
                        false,
                    )?
                    .into_package()?;
                    let config = xui_build::load_config(&project.dir)?
                        .unwrap_or_default()
                        .assets;
                    let generated_dir = generated_dir(&project, &options)?;
                    let pak = Layout::new(&project.dir, &generated_dir, &config).pak;
                    if !pak.is_file() {
                        return Err(CliError::Usage(format!(
                            "no asset package at {} yet; build one with `cargo xui assets pack`",
                            pak.display()
                        )));
                    }
                    pak
                }
            };
            let pak = open_pak(&path)?;
            pak.verify_all().map_err(|source| CliError::Pak {
                path: path.clone(),
                source,
            })?;
            println!(
                "verified {} in {}",
                count(pak.entries().count(), "asset"),
                path.display()
            );
        }
    }
    Ok(())
}

fn pack(args: &PackArgs) -> Result<(Layout, BuildOutput), CliError> {
    let options = args.options();
    let project = resolve_project(options.manifest_path.as_deref(), &options.packages, false)?
        .into_package()?;
    let config = xui_build::load_config(&project.dir)?
        .unwrap_or_default()
        .assets;
    let generated_dir = generated_dir(&project, &options)?;
    Ok(xui_build::generate(
        &project.dir,
        &generated_dir,
        &config,
        Fallbacks::None,
    )?)
}

fn open_pak(path: &Path) -> Result<PakSource, CliError> {
    PakSource::open(path).map_err(|source| CliError::Pak {
        path: path.to_owned(),
        source,
    })
}

/// A table of the entries, sorted by path, and a line of totals.
fn listing(pak: &PakSource) -> String {
    let mut entries: Vec<_> = pak.entries().collect();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    let rows: Vec<[String; 4]> = entries
        .iter()
        .map(|entry| {
            [
                human_size(entry.original_len),
                human_size(entry.stored_len),
                match entry.compression {
                    Compression::None => "none".into(),
                    Compression::Zstd => "zstd".into(),
                },
                entry.path.to_string(),
            ]
        })
        .collect();
    let header = ["SIZE", "STORED", "COMPRESSION", "PATH"];
    let width = |column: usize| {
        rows.iter()
            .map(|row| row[column].len())
            .chain([header[column].len()])
            .max()
            .unwrap_or(0)
    };
    let (size, stored, compression) = (width(0), width(1), width(2));
    let mut output = String::new();
    for row in std::iter::once(header.map(String::from)).chain(rows) {
        output.push_str(&format!(
            "{:>size$}  {:>stored$}  {:<compression$}  {}\n",
            row[0], row[1], row[2], row[3]
        ));
    }
    let original: u64 = entries.iter().map(|entry| entry.original_len).sum();
    let packed: u64 = entries.iter().map(|entry| entry.stored_len).sum();
    output.push_str(&format!(
        "{}, {} ({} stored)\n",
        count(entries.len(), "asset"),
        human_size(original),
        human_size(packed)
    ));
    output
}

/// What `cargo xui` needs from Cargo's arguments.
///
/// Everything after `--` belongs to the program being run and is never
/// looked at, so an application's own `--release` flag is not Cargo's.
#[derive(Debug, Default, PartialEq)]
struct CargoOptions {
    manifest_path: Option<PathBuf>,
    packages: Vec<String>,
    workspace: bool,
    release: bool,
    profile: Option<String>,
    target: Option<String>,
    target_dir: Option<PathBuf>,
    quiet: bool,
    help: bool,
    color: Option<String>,
}

impl CargoOptions {
    fn parse(args: &[OsString]) -> Self {
        let mut options = Self::default();
        let mut args = args.iter().map(|arg| arg.to_string_lossy());
        while let Some(arg) = args.next() {
            if arg == "--" {
                break;
            }
            if let Some(long) = arg.strip_prefix("--") {
                let (name, inline) = match long.split_once('=') {
                    Some((name, value)) => (name, Some(value.to_owned())),
                    None => (long, None),
                };
                let mut value = || inline.clone().or_else(|| args.next().map(Cow::into_owned));
                match name {
                    "manifest-path" => options.manifest_path = value().map(PathBuf::from),
                    "package" => options.packages.extend(value()),
                    "workspace" | "all" => options.workspace = true,
                    "release" => options.release = true,
                    "profile" => options.profile = value(),
                    "target" => {
                        // Cargo accepts several; the first names the directory.
                        let target = value();
                        if options.target.is_none() {
                            options.target = target;
                        }
                    }
                    "target-dir" => options.target_dir = value().map(PathBuf::from),
                    "quiet" => options.quiet = true,
                    "help" => options.help = true,
                    "color" => options.color = value(),
                    _ => {}
                }
            } else if let Some(cluster) = arg.strip_prefix('-')
                && !cluster.is_empty()
            {
                // Short flags cluster: `-rq`. A flag that takes a value ends
                // the cluster -- the rest of it, or the next argument, is the
                // value -- so `-pr` names a package `r` rather than meaning
                // `-p -r`.
                for (index, flag) in cluster.char_indices() {
                    match flag {
                        'r' => options.release = true,
                        'q' => options.quiet = true,
                        'h' => options.help = true,
                        'p' | 'j' | 'F' | 'Z' | 'C' => {
                            let rest = &cluster[index + flag.len_utf8()..];
                            let rest = rest.strip_prefix('=').unwrap_or(rest);
                            let value = if rest.is_empty() {
                                args.next().map(Cow::into_owned)
                            } else {
                                Some(rest.to_owned())
                            };
                            if flag == 'p' {
                                options.packages.extend(value);
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
        options
    }

    /// The directory under the target directory that the profile builds
    /// into. Cargo's built-in profiles do not all use their own name.
    fn profile_dir(&self) -> String {
        match self.profile.as_deref() {
            Some("dev" | "test") => "debug".into(),
            Some("release" | "bench") => "release".into(),
            Some(custom) => custom.into(),
            None if self.release => "release".into(),
            None => "debug".into(),
        }
    }

    fn target(&self) -> Option<String> {
        self.target
            .clone()
            .or_else(|| env::var("CARGO_BUILD_TARGET").ok())
    }
}

/// The package whose assets are being built.
#[derive(Debug)]
struct Project {
    name: String,
    dir: PathBuf,
    manifest: PathBuf,
    target_directory: PathBuf,
    /// Whether its build script packs the assets, through `xui-build`.
    uses_build_script: bool,
    /// Where `xui-build` would be found: beside the `xui` the package
    /// depends on by path, when it does.
    xui_build_path: Option<PathBuf>,
}

enum Resolution {
    Package(Project),
    /// No package can be chosen, and the reason, for commands that need one.
    /// Forwarded Cargo commands run without preparing anything instead.
    Unresolved(String),
}

impl Resolution {
    fn into_package(self) -> Result<Project, CliError> {
        match self {
            Self::Package(project) => Ok(project),
            Self::Unresolved(reason) => Err(CliError::Usage(reason)),
        }
    }
}

#[derive(Debug)]
struct Member {
    name: String,
    dir: PathBuf,
    manifest: PathBuf,
    has_config: bool,
    uses_build_script: bool,
    xui_build_path: Option<PathBuf>,
}

impl Member {
    /// Set up for XUI assets, by either route.
    fn is_app(&self) -> bool {
        self.has_config || self.uses_build_script
    }
}

/// Finds the package the way Cargo would for the same arguments, then picks
/// the one that owns the assets.
fn resolve_project(
    manifest_path: Option<&Path>,
    packages: &[String],
    workspace: bool,
) -> Result<Resolution, CliError> {
    let manifest = match manifest_path {
        Some(path) => path.to_owned(),
        None => find_manifest(&env::current_dir()?)?,
    };
    let manifest = manifest.canonicalize().map_err(file_error(&manifest))?;
    let metadata = cargo_metadata(&manifest)?;
    let mut own = None;
    let mut members = Vec::with_capacity(metadata.packages.len());
    for package in metadata.packages {
        // Canonical on both sides: a temporary directory behind a symlink
        // would otherwise never match.
        let package_manifest = package
            .manifest_path
            .canonicalize()
            .unwrap_or(package.manifest_path);
        let dir = package_manifest
            .parent()
            .expect("a manifest path has a parent")
            .to_owned();
        if package_manifest == manifest {
            own = Some(members.len());
        }
        let uses_build_script = package.dependencies.iter().any(|dependency| {
            dependency.name == BUILD_CRATE && dependency.kind.as_deref() == Some("build")
        });
        let xui_build_path = package
            .dependencies
            .iter()
            .find(|dependency| dependency.name == "xui")
            .and_then(|dependency| dependency.path.as_deref()?.parent())
            .map(|parent| parent.join(BUILD_CRATE))
            .filter(|path| path.join("Cargo.toml").is_file());
        members.push(Member {
            has_config: dir.join(CONFIG_FILE).is_file(),
            name: package.name,
            dir,
            manifest: package_manifest,
            uses_build_script,
            xui_build_path,
        });
    }

    let candidates: Vec<&Member> = if !packages.is_empty() {
        let names: Vec<&str> = packages.iter().map(|spec| package_name(spec)).collect();
        let matching: Vec<_> = members
            .iter()
            .filter(|member| names.contains(&member.name.as_str()))
            .collect();
        if matching.is_empty() {
            return Ok(Resolution::Unresolved(format!(
                "no workspace member is named {}",
                names.join(" or ")
            )));
        }
        matching
    } else if let Some(own) = own.filter(|_| !workspace) {
        vec![&members[own]]
    } else {
        // A workspace root with no package of its own, or `--workspace`.
        members.iter().collect()
    };

    Ok(match select(&candidates)? {
        Some(member) => Resolution::Package(Project {
            name: member.name.clone(),
            dir: member.dir.clone(),
            manifest: member.manifest.clone(),
            target_directory: metadata.target_directory,
            uses_build_script: member.uses_build_script,
            xui_build_path: member.xui_build_path.clone(),
        }),
        None => Resolution::Unresolved(format!(
            "could not tell which package's assets to prepare: none of {} is set up for XUI \
             assets; run from the package's directory, or pass -p",
            candidates
                .iter()
                .map(|member| member.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    })
}

/// Picks the package `cargo xui` prepares assets for.
///
/// Among several candidates, the one set up for XUI assets is it. A lone
/// candidate needs no setup.
fn select<'a>(candidates: &[&'a Member]) -> Result<Option<&'a Member>, CliError> {
    let apps: Vec<&Member> = candidates
        .iter()
        .copied()
        .filter(|member| member.is_app())
        .collect();
    match (apps.as_slice(), candidates) {
        ([member], _) | ([], [member]) => Ok(Some(member)),
        ([], _) => Ok(None),
        (several, _) => Err(CliError::Usage(format!(
            "{} packages are set up for XUI assets ({}); pick one with -p",
            several.len(),
            several
                .iter()
                .map(|member| member.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// The name in a package ID spec: `name`, `name@1.0`, or a URL ending in
/// `#name@1.0`.
fn package_name(spec: &str) -> &str {
    let spec = spec.rsplit('#').next().unwrap_or(spec);
    spec.split('@').next().unwrap_or(spec)
}

fn find_manifest(start: &Path) -> Result<PathBuf, CliError> {
    start
        .ancestors()
        .map(|directory| directory.join("Cargo.toml"))
        .find(|manifest| manifest.is_file())
        .ok_or_else(|| {
            CliError::Usage(format!(
                "could not find Cargo.toml in {} or any parent directory",
                start.display()
            ))
        })
}

fn target_directory(project: &Project, options: &CargoOptions) -> Result<PathBuf, CliError> {
    Ok(match &options.target_dir {
        Some(directory) if directory.is_absolute() => directory.clone(),
        Some(directory) => env::current_dir()?.join(directory),
        None => project.target_directory.clone(),
    })
}

/// Per package: two applications in one workspace share a target directory,
/// and must not overwrite each other's bundle.
fn generated_dir(project: &Project, options: &CargoOptions) -> Result<PathBuf, CliError> {
    Ok(target_directory(project, options)?
        .join("xui")
        .join(options.profile_dir())
        .join(&project.name))
}

struct Prepared {
    layout: Layout,
    /// For `XUI_ASSETS_BOOTSTRAP`, when no build script provides one.
    bootstrap: Option<PathBuf>,
    /// The directory to mount over the bundle while running, when enabled
    /// and present.
    dev_directory: Option<PathBuf>,
    external: Option<External>,
}

struct External {
    /// Beside the executable Cargo is about to build.
    destination: PathBuf,
}

/// Everything a forwarded Cargo command needs around the build itself.
///
/// A package whose build script packs its assets needs nothing generated
/// here, unless its package is external: then a copy is packed to put
/// beside the executable, which a build script cannot reach.
fn prepare(project: &Project, options: &CargoOptions) -> Result<Prepared, CliError> {
    let config = xui_build::load_config(&project.dir)?
        .unwrap_or_default()
        .assets;
    let generated_dir = generated_dir(project, options)?;
    let external = config.bundle == BundleMode::External;
    let layout = if !project.uses_build_script || external {
        let (layout, build) =
            xui_build::generate(&project.dir, &generated_dir, &config, Fallbacks::None)?;
        if build.written && !options.quiet {
            status(
                color_stderr(options.color.as_deref()),
                "Packing",
                format_args!(
                    "{} ({}) for {}",
                    count(build.asset_count, "asset"),
                    human_size(build.source_bytes),
                    project.name
                ),
            );
        }
        layout
    } else {
        Layout::new(&project.dir, &generated_dir, &config)
    };

    Ok(Prepared {
        bootstrap: (!project.uses_build_script).then(|| layout.bootstrap.clone()),
        dev_directory: dev_directory(&config, &layout),
        external: external.then(|| {
            let mut directory = target_directory(project, options)
                .unwrap_or_else(|_| project.target_directory.clone());
            if let Some(target) = options.target() {
                directory.push(target);
            }
            directory.push(options.profile_dir());
            External {
                destination: directory.join(&config.output),
            }
        }),
        layout,
    })
}

fn dev_directory(config: &AssetsConfig, layout: &Layout) -> Option<PathBuf> {
    (config.dev_directory && layout.source.is_dir()).then(|| layout.source.clone())
}

fn copy_if_changed(from: &Path, to: &Path) -> Result<(), CliError> {
    let contents = fs::read(from).map_err(file_error(from))?;
    xui_build::write_if_changed(to, contents).map_err(file_error(to))?;
    Ok(())
}

#[derive(Deserialize)]
struct CargoMetadata {
    packages: Vec<MetadataPackage>,
    target_directory: PathBuf,
}

#[derive(Deserialize)]
struct MetadataPackage {
    name: String,
    manifest_path: PathBuf,
    #[serde(default)]
    dependencies: Vec<MetadataDependency>,
}

#[derive(Deserialize)]
struct MetadataDependency {
    name: String,
    /// `None` for a normal dependency, `"build"` or `"dev"` otherwise.
    kind: Option<String>,
    /// Set for a path dependency.
    #[serde(default)]
    path: Option<PathBuf>,
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
        .output()
        .map_err(CliError::Spawn)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        let stderr = stderr.strip_prefix("error: ").unwrap_or(stderr);
        return Err(CliError::Metadata(format!(
            "`cargo metadata` failed for {}: {stderr}",
            manifest.display()
        )));
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn cargo_executable() -> OsString {
    env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

/// Whether to colour stderr, following the same switches Cargo does.
fn color_stderr(choice: Option<&str>) -> bool {
    let choice = choice
        .map(str::to_owned)
        .or_else(|| env::var("CARGO_TERM_COLOR").ok());
    match choice.as_deref() {
        Some("always") => true,
        Some("never") => false,
        _ => io::stderr().is_terminal() && env::var_os("NO_COLOR").is_none(),
    }
}

fn paint(color: bool, code: &str, text: &str) -> String {
    if color {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_owned()
    }
}

/// A status line in Cargo's format, so it reads as part of Cargo's output.
fn status(color: bool, verb: &str, message: impl fmt::Display) {
    eprintln!("{} {message}", paint(color, "1;32", &format!("{verb:>12}")));
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;
    use xui_pak::AssetId;

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    fn project(dir: &Path) -> Project {
        Project {
            name: "app".into(),
            dir: dir.to_owned(),
            manifest: dir.join("Cargo.toml"),
            target_directory: dir.join("target"),
            uses_build_script: false,
            xui_build_path: None,
        }
    }

    fn build_script_project(dir: &Path) -> Project {
        Project {
            uses_build_script: true,
            ..project(dir)
        }
    }

    fn entries(pak: &Path) -> usize {
        PakSource::open(pak).unwrap().entries().count()
    }

    fn quiet() -> CargoOptions {
        CargoOptions {
            quiet: true,
            ..CargoOptions::default()
        }
    }

    #[test]
    fn cargo_options_follow_cargo_flag_syntax() {
        let options = CargoOptions::parse(&os(&[
            "-rq",
            "--package=app",
            "-p",
            "second",
            "-pthird",
            "--target",
            "wasm32-unknown-unknown",
            "--target=ignored",
            "--manifest-path",
            "app/Cargo.toml",
            "--color=never",
        ]));
        assert!(options.release && options.quiet);
        assert_eq!(options.packages, ["app", "second", "third"]);
        assert_eq!(options.target.as_deref(), Some("wasm32-unknown-unknown"));
        assert_eq!(options.manifest_path, Some(PathBuf::from("app/Cargo.toml")));
        assert_eq!(options.color.as_deref(), Some("never"));
        assert_eq!(options.profile_dir(), "release");

        // `-pr` is the package `r`, not `-p -r`; `-j 4` consumes its value.
        let options = CargoOptions::parse(&os(&["-pr", "-j", "4", "--workspace"]));
        assert_eq!(options.packages, ["r"]);
        assert!(!options.release && options.workspace);
    }

    #[test]
    fn cargo_options_do_not_inspect_application_arguments() {
        let options = CargoOptions::parse(&os(&["--", "--release", "-p", "x", "--help"]));
        assert_eq!(options, CargoOptions::default());
        assert_eq!(options.profile_dir(), "debug");
    }

    #[test]
    fn profiles_map_to_cargo_output_directories() {
        let dir = |args: &[&str]| CargoOptions::parse(&os(args)).profile_dir();
        assert_eq!(dir(&[]), "debug");
        assert_eq!(dir(&["--profile", "dev"]), "debug");
        assert_eq!(dir(&["--profile=test"]), "debug");
        assert_eq!(dir(&["--profile", "bench"]), "release");
        assert_eq!(dir(&["--profile", "dist"]), "dist");
    }

    #[test]
    fn help_is_recognised_so_nothing_is_prepared_for_it() {
        assert!(CargoOptions::parse(&os(&["--help"])).help);
        assert!(CargoOptions::parse(&os(&["-h"])).help);
    }

    fn member(name: &str, has_config: bool) -> Member {
        Member {
            name: name.into(),
            dir: PathBuf::from(name),
            manifest: PathBuf::from(name).join("Cargo.toml"),
            has_config,
            uses_build_script: false,
            xui_build_path: None,
        }
    }

    #[test]
    fn the_member_with_a_config_is_selected() {
        let (app, lib, other) = (
            member("app", true),
            member("lib", false),
            member("other", true),
        );
        let name = |members: &[&Member]| select(members).unwrap().map(|m| m.name.clone());
        assert_eq!(name(&[&lib, &app]), Some("app".into()));
        // A build script calling xui-build counts as set up, config or not.
        let scripted = Member {
            uses_build_script: true,
            ..member("scripted", false)
        };
        assert_eq!(name(&[&lib, &scripted]), Some("scripted".into()));
        // A lone package needs no config; several without one are ambiguous.
        assert_eq!(name(&[&lib]), Some("lib".into()));
        assert_eq!(name(&[&lib, &member("lib2", false)]), None);
        let error = select(&[&app, &other]).unwrap_err().to_string();
        assert!(
            error.contains("app, other") && error.contains("-p"),
            "{error}"
        );
    }

    #[test]
    fn package_specs_reduce_to_names() {
        assert_eq!(package_name("app"), "app");
        assert_eq!(package_name("app@1.2.0"), "app");
        assert_eq!(package_name("path+file:///work/app#app@0.1.0"), "app");
    }

    #[test]
    fn manifests_are_found_from_subdirectories() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("Cargo.toml"), "").unwrap();
        fs::create_dir_all(temp.path().join("src/deep")).unwrap();
        assert_eq!(
            find_manifest(&temp.path().join("src/deep")).unwrap(),
            temp.path().join("Cargo.toml")
        );
        let empty = tempfile::tempdir().unwrap();
        let error = find_manifest(empty.path()).unwrap_err();
        assert!(error.to_string().contains("could not find Cargo.toml"));
    }

    #[test]
    fn a_project_without_config_or_assets_gets_an_empty_bundle() {
        let temp = tempfile::tempdir().unwrap();
        let prepared = prepare(&project(temp.path()), &quiet()).unwrap();
        assert_eq!(entries(&prepared.layout.pak), 0);
        assert!(prepared.dev_directory.is_none());
        let bootstrap = prepared.bootstrap.unwrap();
        assert!(bootstrap.is_file());
        // Keyed by package, so two applications sharing a target directory
        // keep separate bundles.
        assert!(bootstrap.starts_with(temp.path().join("target/xui/debug/app")));
    }

    #[test]
    fn a_build_script_package_gets_only_the_runtime_side() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("assets")).unwrap();
        fs::write(temp.path().join("assets/a.txt"), b"a").unwrap();

        // Embedded: the build script generated everything, so nothing is
        // packed here and no bootstrap is handed to Cargo.
        let prepared = prepare(&build_script_project(temp.path()), &quiet()).unwrap();
        assert!(prepared.bootstrap.is_none());
        assert!(!prepared.layout.pak.exists());
        assert_eq!(prepared.dev_directory, Some(temp.path().join("assets")));

        // External: packed here too, to be copied beside the executable.
        fs::write(
            temp.path().join(CONFIG_FILE),
            "[assets]\nbundle = \"external\"\n",
        )
        .unwrap();
        let prepared = prepare(&build_script_project(temp.path()), &quiet()).unwrap();
        assert!(prepared.bootstrap.is_none());
        assert_eq!(entries(&prepared.layout.pak), 1);
        assert!(prepared.external.is_some());
    }

    #[test]
    fn default_config_packs_the_assets_directory_and_mounts_it() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("assets/data")).unwrap();
        fs::write(temp.path().join("assets/data/app.bin"), b"data").unwrap();
        let prepared = prepare(&project(temp.path()), &quiet()).unwrap();
        assert_eq!(entries(&prepared.layout.pak), 1);
        assert_eq!(prepared.dev_directory, Some(temp.path().join("assets")));
        assert_eq!(
            AssetId::from_path("data/app.bin").unwrap(),
            PakSource::open(&prepared.layout.pak)
                .unwrap()
                .entries()
                .next()
                .unwrap()
                .id
        );
    }

    #[test]
    fn config_mistakes_are_reported_with_the_file() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join(CONFIG_FILE),
            "[assets]\ndev_directroy = false\n",
        )
        .unwrap();
        let error = prepare(&project(temp.path()), &quiet())
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains(CONFIG_FILE) && error.contains("dev_directroy"),
            "{error}"
        );

        fs::write(
            temp.path().join(CONFIG_FILE),
            "[assets]\nsource = \"art\"\n",
        )
        .unwrap();
        let error = prepare(&project(temp.path()), &quiet())
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("assets.source = \"art\""), "{error}");
    }

    #[test]
    fn external_packages_go_beside_the_executable_cargo_builds() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join(CONFIG_FILE),
            "[assets]\nbundle = \"external\"\n",
        )
        .unwrap();
        let destination = |args: &[&str]| {
            let options = CargoOptions {
                quiet: true,
                ..CargoOptions::parse(&os(args))
            };
            prepare(&project(temp.path()), &options)
                .unwrap()
                .external
                .unwrap()
                .destination
        };
        let target = temp.path().join("target");
        assert_eq!(destination(&[]), target.join("debug/assets.xpak"));
        assert_eq!(destination(&["-r"]), target.join("release/assets.xpak"));
        assert_eq!(
            destination(&["--release", "--target", "x86_64-pc-windows-msvc"]),
            target.join("x86_64-pc-windows-msvc/release/assets.xpak")
        );
    }

    #[test]
    fn listings_are_sorted_with_units_and_totals() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("assets")).unwrap();
        fs::write(temp.path().join("assets/b.png"), vec![0; 2048]).unwrap();
        fs::write(temp.path().join("assets/a.txt"), b"hello").unwrap();
        let prepared = prepare(&project(temp.path()), &quiet()).unwrap();
        let listing = listing(&PakSource::open(&prepared.layout.pak).unwrap());
        let lines: Vec<&str> = listing.lines().collect();
        assert!(
            lines[0].contains("SIZE") && lines[0].ends_with("PATH"),
            "{listing}"
        );
        assert!(
            lines[1].ends_with("a.txt") && lines[2].ends_with("b.png"),
            "{listing}"
        );
        assert!(
            lines[2].contains("2.0 KiB") && lines[2].contains("none"),
            "{listing}"
        );
        assert_eq!(lines[3], "2 assets, 2.0 KiB (2.0 KiB stored)");
    }

    #[test]
    fn sizes_and_counts_read_naturally() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1536), "1.5 KiB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(count(1, "asset"), "1 asset");
        assert_eq!(count(3, "asset"), "3 assets");
    }

    fn parse(args: &[&str]) -> CliCommand {
        let mut full = os(&["cargo-xui", "xui"]);
        full.extend(os(args));
        Cli::try_parse_from(normalize_args(full)).unwrap().command
    }

    #[test]
    fn listed_subcommands_forward_arguments_verbatim() {
        let CliCommand::Build(CargoArgs { args }) =
            parse(&["build", "--help", "-r", "--", "--release"])
        else {
            panic!("expected build");
        };
        assert_eq!(args, os(&["--help", "-r", "--", "--release"]));
    }

    #[test]
    fn other_cargo_subcommands_are_forwarded() {
        let CliCommand::External(args) = parse(&["nextest", "run", "-p", "app"]) else {
            panic!("expected a forwarded subcommand");
        };
        assert_eq!(args, os(&["nextest", "run", "-p", "app"]));
    }

    #[test]
    fn asset_commands_take_a_package_file_or_project_options() {
        let CliCommand::Assets(AssetsCommand {
            action: AssetAction::Verify(args),
        }) = parse(&["assets", "verify", "shipped.xpak"])
        else {
            panic!("expected assets verify");
        };
        assert_eq!(args.pak, Some(PathBuf::from("shipped.xpak")));

        let CliCommand::Assets(AssetsCommand {
            action: AssetAction::List(args),
        }) = parse(&["assets", "list", "-r", "--manifest-path", "app/Cargo.toml"])
        else {
            panic!("expected assets list");
        };
        assert_eq!(args.pak, None);
        assert!(args.pack.release);
        assert_eq!(
            args.pack.project.manifest_path,
            Some(PathBuf::from("app/Cargo.toml"))
        );
    }
}
