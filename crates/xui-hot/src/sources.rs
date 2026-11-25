//! Which crate a changed file is in, and so what a change takes: a patch
//! (the binary crate's sources), a restart (any other crate's, a manifest,
//! a build script), or nothing.
//!
//! The binary crate's files, and the other workspace crates', are what
//! rustc says each crate read: the dep-info (`.d`) its recorded call wrote.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::shim::RustcCall;

/// The files of the crates a dev build compiled.
#[derive(Debug, Default)]
pub struct Sources {
    /// The binary crate's: what a patch applies.
    pub bin: HashSet<PathBuf>,
    /// Every other recorded crate's, to its name.
    pub others: HashMap<PathBuf, String>,
    /// The manifests, lockfiles and build scripts as the build found
    /// them: a write that leaves one as it was (cargo rewrites the
    /// lockfile as it builds) is no change.
    pub built: HashMap<PathBuf, Vec<u8>>,
}

/// Whether `name` is a file cargo reads to plan a build.
fn is_build_file(name: &str) -> bool {
    matches!(name, "Cargo.toml" | "Cargo.lock" | "build.rs")
}

impl Sources {
    /// Reads the dep-info of every rustc call recorded in `dir`; the one
    /// of `bin_record` is the binary crate's.
    pub fn load(dir: &Path, bin_record: &Path) -> Self {
        let mut sources = Self::default();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return sources;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !(name.starts_with("rustc-") && name.ends_with(".json")) {
                continue;
            }
            let Some(call) = std::fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<RustcCall>(&bytes).ok())
            else {
                continue;
            };
            let files = crate_files(&call);
            if path == bin_record {
                sources.bin.extend(files);
            } else {
                let krate = call.arg_value("--crate-name").unwrap_or("?").to_owned();
                for file in files {
                    sources.others.entry(file).or_insert_with(|| krate.clone());
                }
            }
        }
        sources
    }

    /// Records the build files of `roots` (package and workspace
    /// directories) as they are now: what the build just read.
    pub fn remember_build_files(&mut self, roots: &[&Path]) {
        for root in roots {
            for name in ["Cargo.toml", "Cargo.lock", "build.rs"] {
                let path = canonical(&root.join(name));
                if let Ok(bytes) = std::fs::read(&path) {
                    self.built.insert(path, bytes);
                }
            }
        }
    }
}

/// What a change is.
#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    /// Files of the binary crate: patch it.
    Patch(Vec<PathBuf>),
    /// Something a patch cannot carry, and why.
    Restart(String),
    /// Nothing a build reads.
    Ignore,
}

/// What a change of `paths` takes. `package_root` is the binary's
/// package's directory; `show` makes a path short for messages.
pub fn classify(
    paths: &[PathBuf],
    sources: &Sources,
    package_root: &Path,
    bin: &str,
    show: impl Fn(&Path) -> String,
) -> Change {
    let mut patch = Vec::new();
    for path in paths {
        let path = canonical(path);
        // Another crate's first, even if the binary crate reads it too
        // (`include!`d by both): a patch would leave that crate as it was.
        if let Some(krate) = sources.others.get(&path) {
            return Change::Restart(format!(
                "{} is in `{krate}`; a patch carries only `{bin}`",
                show(&path)
            ));
        }
        if sources.bin.contains(&path) {
            patch.push(path);
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if is_build_file(name) {
            let same = sources
                .built
                .get(&path)
                .is_some_and(|built| std::fs::read(&path).is_ok_and(|now| &now == built));
            if same {
                continue;
            }
            return Change::Restart(format!("{} changed", show(&path)));
        }
        // A Rust file no crate read: in this package, most likely a module
        // not declared yet (declaring it patches); in another package, one
        // of a crate we have no record of.
        if is_source(&path) && !path.starts_with(package_root) {
            return Change::Restart(format!(
                "{} is outside `{bin}`; a patch carries only `{bin}`",
                show(&path)
            ));
        }
    }
    patch.sort();
    patch.dedup();
    if patch.is_empty() {
        Change::Ignore
    } else {
        Change::Patch(patch)
    }
}

/// A Rust source, not an editor's temporary file next to one
/// (`.name.swp`, `.!123!name.rs`, `name.rs~`).
pub fn is_source(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "rs")
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| !name.starts_with('.') && !name.starts_with('~'))
        && path.exists()
}

/// `path` with symbolic links resolved, as the file watcher reports it;
/// for a path that does not exist (yet), its nearest existing ancestor's.
pub fn canonical(path: &Path) -> PathBuf {
    if let Ok(resolved) = std::fs::canonicalize(path) {
        return resolved;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) if parent != path => canonical(parent).join(name),
        _ => path.to_owned(),
    }
}

/// The files a recorded rustc call read, from the dep-info it wrote.
fn crate_files(call: &RustcCall) -> Vec<PathBuf> {
    let Some(dep_info) = dep_info_path(call) else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&dep_info) else {
        return Vec::new();
    };
    parse_dep_info(&text)
        .into_iter()
        .map(|file| canonical(&call.cwd.join(file)))
        .collect()
}

/// Where rustc wrote the dep-info: `<out-dir>/<crate><extra-filename>.d`.
fn dep_info_path(call: &RustcCall) -> Option<PathBuf> {
    let name = call.arg_value("--crate-name")?;
    let out_dir = call.arg_value("--out-dir")?;
    let extra = call
        .args
        .iter()
        .find_map(|arg| arg.strip_prefix("extra-filename="))
        .unwrap_or("");
    Some(call.cwd.join(out_dir).join(format!("{name}{extra}.d")))
}

/// The files a dep-info lists: rustc writes each as a rule of its own
/// with no prerequisites, `path:`, spaces in it escaped.
fn parse_dep_info(text: &str) -> Vec<PathBuf> {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.strip_suffix(':'))
        .filter(|path| !path.is_empty())
        .map(|path| PathBuf::from(path.replace("\\ ", " ")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_files_of_a_dep_info() {
        let text = "\
/t/deps/app-1.d: src/main.rs src/my\\ ui.rs

/t/deps/app-1: src/main.rs src/my\\ ui.rs

src/main.rs:
src/my\\ ui.rs:

# env-dep:CARGO_PKG_NAME=app
";
        assert_eq!(
            parse_dep_info(text),
            [PathBuf::from("src/main.rs"), PathBuf::from("src/my ui.rs")]
        );
    }

    #[test]
    fn finds_the_dep_info_of_a_call() {
        let call = RustcCall {
            rustc: "rustc".into(),
            args: [
                "--crate-name",
                "app",
                "--out-dir",
                "/t/debug/deps",
                "-C",
                "extra-filename=-0a1b",
            ]
            .map(String::from)
            .to_vec(),
            envs: Vec::new(),
            cwd: "/w".into(),
        };
        assert_eq!(
            dep_info_path(&call),
            Some(PathBuf::from("/t/debug/deps/app-0a1b.d"))
        );
    }

    #[test]
    fn resolves_links_of_paths_not_made_yet() {
        let dir = canonical(&std::env::temp_dir());
        assert_eq!(
            canonical(&std::env::temp_dir().join("xui-hot-none/target/debug")),
            dir.join("xui-hot-none/target/debug")
        );
    }

    #[test]
    fn tells_what_a_change_takes() {
        let dir = std::env::temp_dir().join(format!("xui-hot-sources-{}", std::process::id()));
        let package = dir.join("app");
        let other = dir.join("widgets");
        for d in [package.join("src"), other.join("src")] {
            std::fs::create_dir_all(d).unwrap();
        }
        let main = package.join("src/main.rs");
        let new_module = package.join("src/later.rs");
        let lib = other.join("src/lib.rs");
        let stray = other.join("src/stray.rs");
        for f in [&main, &new_module, &lib, &stray] {
            std::fs::write(f, "").unwrap();
        }
        let (main, new_module, lib, stray) = (
            canonical(&main),
            canonical(&new_module),
            canonical(&lib),
            canonical(&stray),
        );
        let package = canonical(&package);
        std::fs::write(package.join("Cargo.lock"), "# v1").unwrap();
        let shared = canonical(&package.join("src/shared.rs"));
        std::fs::write(&shared, "").unwrap();
        let mut sources = Sources {
            bin: [main.clone(), shared.clone()].into(),
            others: [
                (lib.clone(), "widgets".to_owned()),
                (shared.clone(), "widgets".to_owned()),
            ]
            .into(),
            built: HashMap::new(),
        };
        sources.remember_build_files(&[&package]);
        let sources = sources;
        let show = |p: &Path| p.file_name().unwrap().to_string_lossy().into_owned();
        let classify = |paths: &[PathBuf]| classify(paths, &sources, &package, "app", show);

        assert_eq!(
            classify(std::slice::from_ref(&main)),
            Change::Patch(vec![main.clone()])
        );
        assert_eq!(classify(std::slice::from_ref(&new_module)), Change::Ignore);
        assert_eq!(
            classify(&[main.clone(), lib.clone()]),
            Change::Restart("lib.rs is in `widgets`; a patch carries only `app`".into())
        );
        assert!(matches!(classify(&[stray]), Change::Restart(_)));
        // Read by the binary crate and another: that one needs building.
        assert_eq!(
            classify(&[shared]),
            Change::Restart("shared.rs is in `widgets`; a patch carries only `app`".into())
        );
        assert_eq!(
            classify(&[package.join("Cargo.toml")]),
            Change::Restart("Cargo.toml changed".into())
        );
        assert_eq!(classify(&[package.join("assets/logo.png")]), Change::Ignore);
        // Written as the build found it: no change.
        let lock = package.join("Cargo.lock");
        std::fs::write(&lock, "# v1").unwrap();
        assert_eq!(classify(std::slice::from_ref(&lock)), Change::Ignore);
        std::fs::write(&lock, "# v2").unwrap();
        assert_eq!(
            classify(&[lock]),
            Change::Restart("Cargo.lock changed".into())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
