//! Hot patching (feature `hotpatch`, debug builds run by `cargo xui dev`):
//! what the runtime needs from subsecond.
//!
//! Subsecond's jump table maps the *original* binary's function addresses to
//! the newest patch's. Code mounted by an earlier patch holds that patch's
//! addresses, which the next table does not know; [`resolve`] maps them back
//! to the original first, so a function pointer taken by any version of the
//! code resolves to the newest.
//!
//! Patches come from `cargo xui dev` over TCP ([`connect`]; the protocol is
//! `xui-hot`'s `protocol` module). An app launched some other way never
//! connects, and runs as it was built.
#![cfg(feature = "hotpatch")]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once, OnceLock};

static PATCHED: AtomicBool = AtomicBool::new(false);
static ANY_PATCH: AtomicBool = AtomicBool::new(false);
static WAKE: OnceLock<Arc<dyn Fn() + Send + Sync>> = OnceLock::new();
static REGISTER: Once = Once::new();
/// The components the patches since the last reload ask to mount again,
/// by function name: code they call changed, though they did not.
static REMOUNT: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// A patch's address → the original binary's, for every patch applied.
static ORIGINAL: Mutex<Option<HashMap<u64, u64>>> = Mutex::new(None);

/// Starts listening for patches; once per process.
pub(crate) fn register() {
    REGISTER.call_once(|| {
        connect();
        subsecond::register_handler(Arc::new(|| {
            let started = std::time::Instant::now();
            // Safety: read right after the patch was committed, and not kept.
            if let Some(table) = unsafe { subsecond::get_jump_table() } {
                let mut original = ORIGINAL.lock().unwrap();
                let original = original.get_or_insert_with(HashMap::new);
                for (&old, &new) in &table.map {
                    original.insert(new, old);
                }
                if debug() {
                    eprintln!(
                        "xui hotpatch: table of {} functions, {} patch addresses known",
                        table.map.len(),
                        original.len()
                    );
                }
            }
            if debug() {
                eprintln!("xui hotpatch: handler took {:?}", started.elapsed());
            }
            ANY_PATCH.store(true, Ordering::Release);
            PATCHED.store(true, Ordering::Release);
            if let Some(wake) = WAKE.get() {
                wake();
            }
        }));
    });
}

pub(crate) fn set_wake(wake: Arc<dyn Fn() + Send + Sync>) {
    let _ = WAKE.set(wake);
}

/// The components to mount again for code they call: asked for since the
/// last call.
pub(crate) fn take_remount() -> std::collections::HashSet<String> {
    std::mem::take(&mut *REMOUNT.lock().unwrap())
        .into_iter()
        .collect()
}

/// Whether a patch arrived since the last call.
pub(crate) fn take_patched() -> bool {
    PATCHED.swap(false, Ordering::AcqRel)
}

/// Whether the newest patch knows function pointer `f`: false when the
/// patch renamed it (Rust numbers a module's `impl` blocks, and the number
/// is in the symbol, so an `impl` added above shifts the ones below), and
/// calling it would run the version that took it rather than the newest.
pub(crate) fn resolves<F: Copy>(f: F) -> bool {
    if !ANY_PATCH.load(Ordering::Acquire) {
        return true;
    }
    lookup(f).is_some()
}

/// `f`'s address in the newest patch, if the patch has it.
fn lookup<F: Copy>(f: F) -> Option<u64> {
    assert_eq!(size_of::<F>(), size_of::<usize>(), "not a fn pointer");
    // Safety: `F` is a fn pointer, the size of an address.
    let address = unsafe { std::mem::transmute_copy::<F, usize>(&f) } as u64;
    let original = ORIGINAL
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|map| map.get(&address).copied())
        .unwrap_or(address);
    // Safety: the table is not kept past this read.
    let newest =
        unsafe { subsecond::get_jump_table() }.and_then(|table| table.map.get(&original).copied());
    if newest.is_none() && debug() {
        eprintln!(
            "xui hotpatch: no newer version of {address:#x} (original {original:#x}, {})",
            if original == address {
                "from the binary"
            } else {
                "from a patch"
            }
        );
    }
    newest
}

/// The newest version of function pointer `f`, whichever version of the
/// code took it; `f` itself when the patch does not know it.
///
/// `F` must be a plain `fn` pointer. The newest version keeps `f`'s
/// signature only while the types it names keep their layout; the runtime
/// calls through it only for scopes whose fingerprint did not change.
#[inline]
pub(crate) fn resolve<F: Copy>(f: F) -> F {
    if !ANY_PATCH.load(Ordering::Acquire) {
        return f;
    }
    match lookup(f) {
        // Safety: the table maps a function to its newer version, same signature.
        Some(newest) => unsafe { std::mem::transmute_copy::<usize, F>(&(newest as usize)) },
        None => f,
    }
}

fn debug() -> bool {
    static DEBUG: OnceLock<bool> = OnceLock::new();
    *DEBUG.get_or_init(|| std::env::var_os("XUI_HOTPATCH_DEBUG").is_some())
}

/// The connection to `cargo xui dev`, for what the app reports.
static LINK: Mutex<Option<std::net::TcpStream>> = Mutex::new(None);

/// Connects to `cargo xui dev` at `XUI_HOT_ADDR`, if the app was launched
/// by it, and applies each patch it sends.
///
/// The hello says where `main` is loaded: `cargo xui dev` links each patch
/// against this process's addresses, so a patch is only ever for the
/// process that said hello on this connection.
fn connect() {
    let Ok(address) = std::env::var("XUI_HOT_ADDR") else {
        return;
    };
    let build = std::env::var("XUI_HOT_BUILD").unwrap_or_default();
    let stream = match std::net::TcpStream::connect(&address) {
        Ok(stream) => stream,
        Err(error) => {
            log::warn!("xui hotpatch: cannot reach `cargo xui dev` at {address}: {error}");
            return;
        }
    };
    let Ok(write_half) = stream.try_clone() else {
        return;
    };
    *LINK.lock().unwrap() = Some(write_half);
    send(serde_json::json!({ "hello": {
        "pid": std::process::id(),
        "aslr_reference": subsecond::aslr_reference(),
        "build": build,
    }}));
    let spawned = std::thread::Builder::new()
        .name("xui-hotpatch".into())
        .spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                receive(&line);
            }
        });
    if let Err(error) = spawned {
        log::error!("xui hotpatch: could not start its thread: {error}");
    }
}

/// Handles one message from `cargo xui dev`:
/// `{"patch": {"id", "table", "remount"}}`.
fn receive(line: &str) {
    let read = std::time::Instant::now();
    let message: serde_json::Value = match serde_json::from_str(line) {
        Ok(message) => message,
        Err(error) => {
            log::error!("xui hotpatch: bad message: {error}");
            return;
        }
    };
    let Some(patch) = message.get("patch") else {
        return;
    };
    if debug() {
        eprintln!(
            "xui hotpatch: line of {} bytes read as JSON in {:?}",
            line.len(),
            read.elapsed()
        );
    }
    let id = patch
        .get("id")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    // Before the patch goes in: its handler starts the reload.
    if let Some(names) = patch.get("remount").and_then(serde_json::Value::as_array) {
        REMOUNT.lock().unwrap().extend(
            names
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned),
        );
    }
    let started = std::time::Instant::now();
    let table = patch
        .get("table")
        .cloned()
        .ok_or_else(|| "no jump table".to_owned())
        .and_then(|table| {
            serde_json::from_value::<subsecond::JumpTable>(table).map_err(|e| e.to_string())
        });
    let parsed = started.elapsed();
    // Safety: `cargo xui dev` linked the patch against this process,
    // which said hello on this connection.
    let error = table
        .and_then(|table| unsafe { subsecond::apply_patch(table) }.map_err(|e| format!("{e:?}")))
        .err();
    if debug() {
        eprintln!(
            "xui hotpatch: patch {id}: table read in {parsed:?}, loaded in {:?}",
            started.elapsed() - parsed
        );
    }
    if let Some(error) = &error {
        log::error!("xui hotpatch: patch {id}: {error}");
    }
    send(serde_json::json!({ "applied": { "id": id, "error": error } }));
}

/// Tells `cargo xui dev` what a reload did.
pub(crate) fn report_reload(
    changed: usize,
    called: usize,
    renamed: usize,
    remounted: usize,
    kept: usize,
    micros: u64,
    full: bool,
) {
    send(serde_json::json!({ "reloaded": {
        "changed": changed,
        "called": called,
        "renamed": renamed,
        "remounted": remounted,
        "kept": kept,
        "micros": micros,
        "full": full,
    }}));
}

fn send(message: serde_json::Value) {
    use std::io::Write;
    let mut link = LINK.lock().unwrap();
    let Some(stream) = link.as_mut() else { return };
    let mut line = message.to_string();
    line.push('\n');
    if stream.write_all(line.as_bytes()).is_err() {
        *link = None;
    }
}
