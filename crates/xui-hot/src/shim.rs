//! The tool as rustc wrapper and as linker, during a dev build.
//!
//! A dev build runs `cargo rustc` with this executable as
//! `RUSTC_WORKSPACE_WRAPPER` and as the binary's `-C linker`, and with
//! [`DIR_ENV`] naming where to record things. The executable then is called
//! in one of two ways, told apart by its first argument:
//!
//! - as the wrapper, `<tool> <path to rustc> <rustc args>`: records the call
//!   ([`RustcCall`]) and runs rustc;
//! - as the linker, `<tool> <linker args>`: records the arguments, then, by
//!   [`LINK_ENV`], links the fat binary ([`crate::link::fat`]) or does
//!   nothing (a thin build only wants the objects).
//!
//! The binary's entry point calls [`run_if_shim`] first.
//!
//! # What a recorded call keeps of the environment
//!
//! Only what cargo set for rustc: `CARGO_PKG_*`, `OUT_DIR`, a build
//! script's `rustc-env`, which `env!` reads. What it inherited from the
//! session `cargo xui dev` runs in (tokens, credentials) is not written
//! down; [`INHERITED_ENV`] says what that was, as hashes, and a thin build
//! inherits it from the session again.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde::{Deserialize, Serialize};

use anyhow::{Context, Result};

/// The directory a dev build records in. Set: this process is a shim.
pub const DIR_ENV: &str = "XUI_HOT_DIR";
/// What the linker shim does: `fat` or `thin`.
pub const LINK_ENV: &str = "XUI_HOT_LINK";
/// The environment `cargo xui dev` runs in, as one hash per variable
/// (name and value), comma-separated: what a recorded call leaves out.
pub const INHERITED_ENV: &str = "XUI_HOT_INHERITED";

/// How cargo ran rustc for one crate: what a thin build runs again.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RustcCall {
    pub rustc: String,
    pub args: Vec<String>,
    /// What cargo added to the environment, or changed (`env!` reads
    /// cargo's variables at compile time); not what it inherited.
    pub envs: Vec<(String, String)>,
    pub cwd: PathBuf,
}

impl RustcCall {
    pub fn arg_value(&self, name: &str) -> Option<&str> {
        let at = self.args.iter().position(|arg| arg == name)?;
        self.args.get(at + 1).map(String::as_str)
    }
}

/// What the linker was called with.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkCall {
    pub args: Vec<String>,
}

/// The file a crate's rustc call is recorded in.
pub fn rustc_record(dir: &Path, crate_name: &str, crate_type: &str) -> PathBuf {
    dir.join(format!("rustc-{crate_name}-{crate_type}.json"))
}

/// The file the last link of `mode` is recorded in.
pub fn link_record(dir: &Path, mode: &str) -> PathBuf {
    dir.join(format!("link-{mode}.json"))
}

/// Runs as a shim if this process is one: its exit code; `None` otherwise.
pub fn run_if_shim() -> Option<ExitCode> {
    let dir = PathBuf::from(std::env::var_os(DIR_ENV)?);
    let args: Vec<String> = std::env::args().collect();
    let result = if args.get(1).is_some_and(|first| is_rustc(first)) {
        wrap_rustc(&dir, &args[1], &args[2..])
    } else {
        link(&dir, &args[1..])
    };
    Some(match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: xui dev: {error:#}");
            ExitCode::FAILURE
        }
    })
}

fn is_rustc(path: &str) -> bool {
    Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem == "rustc")
}

/// This process's environment, as [`INHERITED_ENV`] says it.
pub fn inherited_env() -> String {
    std::env::vars_os()
        .map(|(key, value)| env_hash(&key, &value))
        .collect::<Vec<_>>()
        .join(",")
}

fn env_hash(key: &std::ffi::OsStr, value: &std::ffi::OsStr) -> String {
    use std::hash::{Hash, Hasher};
    // Fixed keys: the same in `cargo xui dev` and in its shims, which are
    // the same executable.
    let mut hasher = std::hash::DefaultHasher::new();
    key.hash(&mut hasher);
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Of `vars`, what to record: not `inherited` (hashes), not ours, and
/// text (a thin build inherits the rest from the session anyway).
fn recorded_env(
    vars: impl Iterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
    inherited: &str,
) -> Vec<(String, String)> {
    let inherited: std::collections::HashSet<&str> = inherited.split(',').collect();
    vars.filter(|(key, value)| !inherited.contains(env_hash(key, value).as_str()))
        .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
        .filter(|(key, _)| !key.starts_with("XUI_HOT_"))
        .collect()
}

/// Leaves out of the calls recorded in `dir` what this process's
/// environment has: what an older `cargo xui dev`, which recorded the whole
/// environment, wrote down of the session. Cargo does not run rustc again
/// for a crate that is fresh, so such records stay.
pub fn scrub_records(dir: &Path) {
    let inherited = inherited_env();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !(name.starts_with("rustc-") && name.ends_with(".json")) {
            continue;
        }
        let Some(mut call) = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<RustcCall>(&bytes).ok())
        else {
            continue;
        };
        let kept = recorded_env(
            std::mem::take(&mut call.envs)
                .into_iter()
                .map(|(key, value)| (key.into(), value.into())),
            &inherited,
        );
        call.envs = kept;
        if let Ok(bytes) = serde_json::to_vec_pretty(&call) {
            let _ = std::fs::write(&path, bytes);
        }
    }
}

fn wrap_rustc(dir: &Path, rustc: &str, args: &[String]) -> Result<ExitCode> {
    let inherited = std::env::var(INHERITED_ENV).unwrap_or_default();
    let call = RustcCall {
        rustc: rustc.to_owned(),
        args: args.to_vec(),
        envs: recorded_env(std::env::vars_os(), &inherited),
        cwd: std::env::current_dir()?,
    };
    // Probes (`rustc -vV`, `--print`) name no crate.
    if let Some(name) = call.arg_value("--crate-name") {
        let kind = call.arg_value("--crate-type").unwrap_or("lib");
        let record = rustc_record(dir, name, kind);
        std::fs::write(&record, serde_json::to_vec_pretty(&call)?)
            .with_context(|| format!("recording {}", record.display()))?;
    }
    let status = Command::new(rustc)
        .args(args)
        .status()
        .with_context(|| format!("running {rustc}"))?;
    Ok(exit_code(status))
}

fn link(dir: &Path, args: &[String]) -> Result<ExitCode> {
    let mode = std::env::var(LINK_ENV).unwrap_or_else(|_| "fat".into());
    let record = link_record(dir, &mode);
    std::fs::write(
        &record,
        serde_json::to_vec_pretty(&LinkCall {
            args: args.to_vec(),
        })?,
    )
    .with_context(|| format!("recording {}", record.display()))?;
    match mode.as_str() {
        "thin" => {
            // rustc expects an output; the objects are what we came for. A
            // new file, not the old one truncated: that may be running.
            if let Some(out) = output(args) {
                let _ = std::fs::remove_file(out);
                std::fs::write(out, b"")?;
            }
            Ok(ExitCode::SUCCESS)
        }
        _ => crate::link::fat(dir, args).map(exit_code),
    }
}

/// The `-o` argument of a link.
pub fn output(args: &[String]) -> Option<&str> {
    let at = args.iter().position(|arg| arg == "-o")?;
    args.get(at + 1).map(String::as_str)
}

pub(crate) fn exit_code(status: std::process::ExitStatus) -> ExitCode {
    match status.code() {
        Some(0) => ExitCode::SUCCESS,
        Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        None => ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_rustc_from_linker_arguments() {
        assert!(is_rustc("rustc"));
        assert!(is_rustc("/Users/me/.rustup/toolchains/stable/bin/rustc"));
        assert!(!is_rustc("-arch"));
        assert!(!is_rustc("/tmp/deps/main.main.abc-cgu.0.rcgu.o"));
    }

    #[test]
    fn records_what_cargo_set_not_what_it_inherited() {
        let var = |key: &str, value: &str| -> (std::ffi::OsString, std::ffi::OsString) {
            (key.into(), value.into())
        };
        let session = [var("HOME", "/Users/me"), var("API_TOKEN", "secret")];
        let inherited = session
            .iter()
            .map(|(key, value)| env_hash(key, value))
            .collect::<Vec<_>>()
            .join(",");
        let rustc = [
            var("HOME", "/Users/me"),
            var("API_TOKEN", "secret"),
            var("CARGO_PKG_NAME", "app"),
            var("XUI_ASSETS_BOOTSTRAP", "/t/out/bootstrap.rs"),
            var("XUI_HOT_DIR", "/t/xui-hot"),
            // Changed by cargo: recorded.
            var("HOME", "/elsewhere"),
        ];
        let recorded = recorded_env(rustc.into_iter(), &inherited);
        assert_eq!(
            recorded,
            [
                ("CARGO_PKG_NAME".to_owned(), "app".to_owned()),
                (
                    "XUI_ASSETS_BOOTSTRAP".to_owned(),
                    "/t/out/bootstrap.rs".to_owned()
                ),
                ("HOME".to_owned(), "/elsewhere".to_owned()),
            ]
        );
    }

    #[test]
    fn finds_the_link_output() {
        let args = ["-arch", "arm64", "a.o", "-o", "/t/app-123", "-lc"].map(String::from);
        assert_eq!(output(&args), Some("/t/app-123"));
        assert_eq!(output(&args[..3]), None);
    }
}
