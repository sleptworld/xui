//! The two links: the fat binary, once; a patch library, per change.
//!
//! Both drive the platform's C compiler driver (`cc`), as rustc does.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use anyhow::{Context, Result, bail};

/// The target directory, whose rlibs the fat link takes whole.
pub const TARGET_DIR_ENV: &str = "XUI_HOT_TARGET_DIR";

fn linker() -> String {
    std::env::var("XUI_HOT_CC").unwrap_or_else(|_| "cc".into())
}

/// Links the fat binary from the arguments rustc gave the linker.
///
/// What a patch calls must be in the binary, though the app itself never
/// called it: a patch may start using any function of a dependency. So
/// the objects of every rlib built into the target directory go into one
/// archive linked with `-force_load`, and rustc's `-C link-dead-code` (set
/// by the dev build) keeps the binary crate's own. The standard library's
/// rlibs, outside the target directory, are linked as usual.
pub fn fat(dir: &Path, args: &[String]) -> Result<ExitStatus> {
    let target_dir = std::env::var_os(TARGET_DIR_ENV).map(PathBuf::from);
    let mut whole = ar::Builder::new(Vec::new());
    let mut whole_has_members = false;
    // Rlibs with members other than objects (native libraries bundled in)
    // stay on the command line too, for those.
    let mut kept_rlibs = Vec::new();
    for rlib in args.iter().filter(|arg| arg.ends_with(".rlib")) {
        let inside = target_dir
            .as_ref()
            .is_some_and(|target| Path::new(rlib).starts_with(target));
        if !inside {
            kept_rlibs.push(rlib.clone());
            continue;
        }
        let bytes = std::fs::read(rlib).with_context(|| format!("reading {rlib}"))?;
        let mut archive = ar::Archive::new(bytes.as_slice());
        let mut other_members = false;
        while let Some(entry) = archive.next_entry() {
            let mut entry = entry.with_context(|| format!("reading {rlib}"))?;
            let name = String::from_utf8_lossy(entry.header().identifier()).into_owned();
            if name.ends_with(".rmeta") || entry.header().size() == 0 {
                continue;
            }
            if !name.ends_with(".rcgu.o") {
                other_members = true;
                continue;
            }
            let header = entry.header().clone();
            let mut data = Vec::with_capacity(header.size() as usize);
            entry.read_to_end(&mut data)?;
            whole.append(&header, data.as_slice())?;
            whole_has_members = true;
        }
        if other_members {
            kept_rlibs.push(rlib.clone());
        }
    }
    let archive_path = dir.join("libdeps.a");
    std::fs::write(&archive_path, whole.into_inner()?)
        .with_context(|| format!("writing {}", archive_path.display()))?;
    if whole_has_members && cfg!(target_vendor = "apple") {
        // ld64 wants the archive's symbol table.
        let ranlib = Command::new("ranlib").arg(&archive_path).status();
        if !ranlib.is_ok_and(|status| status.success()) {
            bail!("ranlib failed on {}", archive_path.display());
        }
    }

    let mut out: Vec<String> = args
        .iter()
        .filter(|arg| !arg.ends_with(".rlib") && !arg.contains("dead_strip"))
        .cloned()
        .collect();
    let at = out
        .iter()
        .rposition(|arg| arg.ends_with(".o"))
        .map_or(out.len(), |at| at + 1);
    let mut inserted = Vec::new();
    if whole_has_members {
        inserted.push("-Wl,-force_load".to_owned());
        inserted.push(archive_path.display().to_string());
    }
    inserted.extend(kept_rlibs);
    out.splice(at..at, inserted);
    // A patch finds where it was loaded by its own `main`, and the binary's
    // by this one (`subsecond::aslr_reference`).
    out.push("-Wl,-exported_symbol,_main".to_owned());

    let status = Command::new(linker())
        .args(&out)
        .status()
        .with_context(|| format!("running {}", linker()))?;
    if status.success() {
        // Kept by `-C save-temps` for the thin build; the fat one is done.
        for object in args.iter().filter(|arg| arg.ends_with(".rcgu.o")) {
            let _ = std::fs::remove_file(object);
        }
    }
    Ok(status)
}

/// Links a patch library from the binary crate's objects and the stub.
///
/// `original` is what rustc gave the (thin) linker shim: of those, only what
/// says what to link for (`-arch`, `-target`), with what (`-framework`,
/// `-l..`, `-L`) and how (`-m..`) is kept.
pub fn thin(objects: &[PathBuf], original: &[String], out: &Path) -> Result<()> {
    let mut args: Vec<String> = objects.iter().map(|o| o.display().to_string()).collect();
    args.push("-Wl,-dylib".to_owned());
    let mut iter = original.iter().peekable();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-framework" | "-arch" | "-L" | "-target" => {
                if let Some(value) = iter.next() {
                    args.push(arg.clone());
                    args.push(value.clone());
                }
            }
            _ if arg.starts_with("-l") || arg.starts_with("-m") || arg == "-nodefaultlibs" => {
                args.push(arg.clone());
            }
            _ => {}
        }
    }
    args.push("-o".to_owned());
    args.push(out.display().to_string());
    let output = Command::new(linker())
        .args(&args)
        .output()
        .with_context(|| format!("running {}", linker()))?;
    if !output.status.success() || !out.exists() {
        bail!(
            "linking the patch failed:\n{}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}
