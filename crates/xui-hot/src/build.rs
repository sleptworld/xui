//! The builds: what to build, the fat build, a thin build.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use cargo_metadata::diagnostic::{Diagnostic, DiagnosticLevel};
use cargo_metadata::{DependencyKind, Message, MetadataCommand, TargetKind};
use serde::Deserialize;

use crate::link::TARGET_DIR_ENV;
use crate::shim::{self, DIR_ENV, INHERITED_ENV, LINK_ENV, LinkCall, RustcCall};
use crate::term;

/// The binary being developed.
#[derive(Debug, Clone)]
pub struct Target {
    pub package: String,
    pub manifest_path: PathBuf,
    /// The package's directory: changes in it are patched.
    pub root: PathBuf,
    pub workspace_root: PathBuf,
    pub bin: String,
    pub target_dir: PathBuf,
    /// The directories of the local packages the binary is built from, its
    /// own first: what is watched.
    pub local_roots: Vec<PathBuf>,
    /// Turns on hot patching in the app's runtime: `xui-core/hotpatch`, or
    /// `xui/hotpatch` through the facade.
    pub feature: String,
}

impl Target {
    /// rustc's name for the binary crate.
    pub fn crate_name(&self) -> String {
        self.bin.replace('-', "_")
    }

    /// `path` relative to the workspace, for messages.
    pub fn show(&self, path: &Path) -> String {
        path.strip_prefix(&self.workspace_root)
            .unwrap_or(path)
            .display()
            .to_string()
    }
}

/// Finds the binary as cargo would for `cargo run`: the package named, or
/// the one the current directory is in; its only binary, the one named, or
/// its `default-run`.
pub fn resolve(
    manifest_path: Option<&Path>,
    package: Option<&str>,
    bin: Option<&str>,
) -> Result<Target> {
    let mut command = MetadataCommand::new();
    command.no_deps();
    if let Some(path) = manifest_path {
        command.manifest_path(path);
    }
    let metadata = command
        .exec()
        .context("reading the workspace with `cargo metadata`")?;
    let cwd = std::env::current_dir()?;
    let package = match package {
        Some(name) => metadata
            .packages
            .iter()
            .find(|p| p.name.as_str() == name)
            .with_context(|| format!("no package `{name}` in the workspace"))?,
        None => metadata
            .packages
            .iter()
            .filter(|p| {
                p.manifest_path
                    .parent()
                    .is_some_and(|dir| cwd.starts_with(dir))
            })
            .max_by_key(|p| p.manifest_path.components().count())
            .context("not inside a package: pass -p <package>")?,
    };
    let bins: Vec<&str> = package
        .targets
        .iter()
        .filter(|t| t.kind.contains(&TargetKind::Bin))
        .map(|t| t.name.as_str())
        .collect();
    let bin = match bin.or(package.default_run.as_deref()) {
        Some(name) if bins.contains(&name) => name.to_owned(),
        Some(name) => bail!("`{}` has no binary `{name}`", package.name),
        None => match bins.as_slice() {
            [only] => (*only).to_owned(),
            [] => bail!("`{}` has no binary", package.name),
            many => bail!(
                "`{}` has several binaries; pass --bin with one of: {}",
                package.name,
                many.join(", ")
            ),
        },
    };
    let depends_on = |name: &str| {
        package
            .dependencies
            .iter()
            .any(|d| d.kind == DependencyKind::Normal && d.name == name && d.rename.is_none())
    };
    let feature = if depends_on("xui-core") {
        "xui-core/hotpatch"
    } else if depends_on("xui") {
        "xui/hotpatch"
    } else {
        bail!("`{}` depends on neither `xui` nor `xui-core`", package.name)
    };
    // As the file watcher reports paths: with symbolic links resolved
    // (`/tmp` is `/private/tmp` on macOS).
    let canonical = |path: PathBuf| crate::sources::canonical(&path);
    let local_roots = local_roots(&metadata, package)
        .into_iter()
        .map(canonical)
        .collect();
    let manifest_path = PathBuf::from(package.manifest_path.clone());
    Ok(Target {
        local_roots,
        package: package.name.to_string(),
        root: canonical(
            manifest_path
                .parent()
                .context("a manifest without a directory")?
                .to_owned(),
        ),
        manifest_path,
        workspace_root: canonical(metadata.workspace_root.into()),
        bin,
        target_dir: canonical(metadata.target_directory.into()),
        feature: feature.to_owned(),
    })
}

/// The directories of `package` and of the local (path) packages it is
/// built from, normal and build dependencies, transitively through the
/// workspace's members.
fn local_roots(
    metadata: &cargo_metadata::Metadata,
    package: &cargo_metadata::Package,
) -> Vec<PathBuf> {
    let members: std::collections::HashMap<PathBuf, &cargo_metadata::Package> = metadata
        .packages
        .iter()
        .filter_map(|p| Some((p.manifest_path.parent()?.as_std_path().to_owned(), p)))
        .collect();
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut queue = vec![package];
    if let Some(dir) = package.manifest_path.parent() {
        roots.push(dir.as_std_path().to_owned());
    }
    while let Some(next) = queue.pop() {
        for dependency in &next.dependencies {
            if dependency.kind == DependencyKind::Development {
                continue;
            }
            let Some(path) = &dependency.path else {
                continue;
            };
            let path = path.as_std_path().to_owned();
            if roots.contains(&path) {
                continue;
            }
            if let Some(member) = members.get(&path) {
                queue.push(member);
            }
            roots.push(path);
        }
    }
    roots
}

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

/// The fat build: `cargo rustc` with this tool as wrapper and linker. The
/// binary is copied into `dir` and the copy returned, so that builds after
/// it never touch the file the app runs from.
///
/// Shows a spinner with what cargo is compiling, and the diagnostics.
pub fn fat(
    target: &Target,
    dir: &Path,
    tool: &Path,
    cargo_args: &[String],
    envs: &[(OsString, OsString)],
) -> Result<PathBuf> {
    let started = Instant::now();
    let spinner = term::spinner("Building", format!("{} (with hot patching)", target.bin));
    let mut child = Command::new(cargo())
        .arg("rustc")
        .arg("--manifest-path")
        .arg(&target.manifest_path)
        .args(["--bin", &target.bin])
        .args(["--features", &target.feature])
        .args([
            "--message-format",
            "json-diagnostic-rendered-ansi",
            "--quiet",
        ])
        .args(cargo_args)
        .arg("--")
        // The objects, kept for the thin build; every function, kept for patches.
        .args(["-Csave-temps=true", "-Clink-dead-code"])
        .arg(format!("-Clinker={}", tool.display()))
        .env("RUSTC_WORKSPACE_WRAPPER", tool)
        .env(DIR_ENV, dir)
        .env(LINK_ENV, "fat")
        // What the recorded calls leave out: the session's environment.
        .env(INHERITED_ENV, shim::inherited_env())
        .env(TARGET_DIR_ENV, &target.target_dir)
        .envs(envs.iter().cloned())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("running cargo")?;
    // Cargo's own errors (a bad manifest, a failed link) come on stderr.
    let stderr = child.stderr.take().expect("piped");
    let cargo_errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = BufReader::new(stderr).read_to_string(&mut text);
        text
    });
    let mut built = 0;
    let mut succeeded = false;
    // Other crates' warnings are not this app's to fix now; their errors are.
    let mut hidden_warnings = 0;
    let stdout = BufReader::new(child.stdout.take().expect("piped"));
    for message in Message::parse_stream(stdout).flatten() {
        match message {
            Message::CompilerArtifact(artifact) => {
                built += 1;
                spinner.set_message(format!(
                    "{} {}",
                    artifact.target.name,
                    term::paint(term::DIM, format!("({})", term::count(built, "crate")))
                ));
            }
            Message::CompilerMessage(message) => {
                let ours = Path::new(message.target.src_path.as_str()).starts_with(&target.root);
                let error = matches!(
                    message.message.level,
                    DiagnosticLevel::Error | DiagnosticLevel::Ice
                );
                if !ours && !error {
                    hidden_warnings += 1;
                } else if let Some(rendered) = message.message.rendered {
                    term::println(rendered);
                }
            }
            Message::BuildFinished(finished) => succeeded = finished.success,
            _ => {}
        }
    }
    let status = child.wait()?;
    spinner.finish_and_clear();
    let cargo_errors = cargo_errors.join().unwrap_or_default();
    if !status.success() || !succeeded {
        if !cargo_errors.trim().is_empty() {
            term::println(cargo_errors.trim_end());
        }
        bail!("could not build `{}`", target.bin);
    }
    let link: LinkCall = read_json(&shim::link_record(dir, "fat"))
        .context("the build did not link the binary through `cargo xui dev`")?;
    let output = shim::output(&link.args)
        .context("the link had no output")?
        .to_owned();
    let copy = dir.join(&target.bin);
    let _ = std::fs::remove_file(&copy);
    std::fs::copy(&output, &copy).with_context(|| format!("copying {output}"))?;
    let hidden = if hidden_warnings > 0 {
        format!(
            ", {} in other crates hidden",
            term::count(hidden_warnings, "warning")
        )
    } else {
        String::new()
    };
    term::status(
        "Finished",
        format!(
            "`{}` in {} {}",
            target.bin,
            term::duration(started.elapsed()),
            term::paint(
                term::DIM,
                format!("(every dependency linked whole{hidden})")
            )
        ),
    );
    Ok(copy)
}

/// What a thin build made.
pub struct Thin {
    /// The binary crate's objects.
    pub objects: Vec<PathBuf>,
    /// What rustc gave the linker.
    pub link: Vec<String>,
}

/// Compiles the binary crate again, as cargo did in the fat build.
///
/// Diagnostics are printed; `Err` holds a short reason.
pub fn thin(target: &Target, dir: &Path) -> Result<Thin> {
    let record = shim::rustc_record(dir, &target.crate_name(), "bin");
    let call: RustcCall = read_json(&record)
        .with_context(|| format!("no recorded rustc call for `{}`", target.bin))?;
    // The session's environment, as cargo had it, and what cargo set.
    let mut child = Command::new(&call.rustc)
        .args(&call.args)
        .envs(
            call.envs
                .iter()
                .filter(|(key, _)| !key.ends_with("MAKEFLAGS"))
                .cloned(),
        )
        // Cargo's jobserver was the fat build's; this rustc has none.
        .env_remove("CARGO_MAKEFLAGS")
        .env_remove("MAKEFLAGS")
        .env_remove("MFLAGS")
        .env(DIR_ENV, dir)
        .env(LINK_ENV, "thin")
        .current_dir(&call.cwd)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("running {}", call.rustc))?;
    // rustc speaks JSON to cargo: diagnostics and artifact notices.
    for line in BufReader::new(child.stderr.take().expect("piped")).lines() {
        let Ok(line) = line else { break };
        match serde_json::from_str::<Diagnostic>(&line) {
            Ok(diagnostic) => {
                if let Some(rendered) = diagnostic.rendered {
                    term::println(rendered);
                }
            }
            Err(_) if line.starts_with('{') => {}
            Err(_) => term::println(line),
        }
    }
    if !child.wait()?.success() {
        bail!(
            "`{}` does not compile; the app keeps running the last version",
            target.bin
        );
    }
    let link: LinkCall = read_json(&shim::link_record(dir, "thin"))?;
    // The shim's empty stand-in for the binary: gone, so cargo builds the
    // real one next time.
    if let Some(out) = shim::output(&link.args) {
        let _ = std::fs::remove_file(out);
    }
    let objects = link
        .args
        .iter()
        .filter(|arg| arg.ends_with(".rcgu.o"))
        .map(PathBuf::from)
        .collect();
    Ok(Thin {
        objects,
        link: link.args,
    })
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(serde_json::from_slice(&bytes)?)
}
