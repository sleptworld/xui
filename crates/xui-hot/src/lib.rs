//! The toolchain side of hot patching an xui app: `cargo xui dev`.
//!
//! An app built with `xui-core`'s `hotpatch` feature can take a patch while
//! it runs: a dynamic library of its binary crate compiled again, and a jump
//! table from each of its functions' addresses to the new version's
//! ([`subsecond`](https://docs.rs/subsecond) applies it). This crate makes
//! both, and hands them to the app.
//!
//! # How a patch is made
//!
//! 1. **Fat build, once.** `cargo rustc` builds the binary with this tool
//!    as the workspace's rustc wrapper and as the binary's linker
//!    ([`shim`]). The wrapper records how every workspace crate was
//!    compiled; the linker links every dependency's object whole, so a
//!    patch can call any function of them, even one the app never did
//!    ([`link::fat`]).
//! 2. **Thin build, per change.** The binary crate's recorded `rustc`
//!    command runs again; the linker shim only records which objects it
//!    made ([`build::thin`]).
//! 3. **Stub.** What those objects use but do not define is in the running
//!    app: the stub object defines each such symbol at its address there
//!    (functions as a jump to it, statics as the address itself), slid by
//!    where the app's `main` is loaded ([`symbols::stub`]). The binary
//!    crate's own statics, which its objects define again, are first taken
//!    out of them and defined in the stub's mirror instead ([`alias`]).
//! 4. **Patch.** The objects and the stub link into a dynamic library
//!    ([`link::thin`]); its symbols, matched by name with the app's, are the
//!    jump table ([`symbols::jump_table`]).
//!
//! # Talking to the app
//!
//! The app connects to `XUI_HOT_ADDR` over TCP; see [`protocol`].
//!
//! # When it rebuilds instead
//!
//! A patch carries the binary crate's code, nothing else. A change it
//! cannot carry rebuilds and restarts the app, saying why: a file of
//! another crate, a manifest or build script ([`sources`]), or one that
//! reshapes what the running app holds in memory — a type, a static, a
//! trait's methods ([`shapes`]). So is a patch that does not link or load.
//!
//! A component that did not change but calls code that did is mounted
//! again with the patch ([`shapes`] finds it by name): props that read no
//! signal and handlers hold the code they were built with.
//!
//! The binary crate's statics are the app's in every patch: on arm64, the
//! patch's references to them are taken out of its objects and resolved
//! in a section the app maps its own pages over ([`alias`]). Thread-locals
//! (and, on x86_64, every static) start over in each patch, which `dev`
//! warns about.
//!
//! Supported: macOS (aarch64, x86_64).

pub mod alias;
pub mod build;
pub mod keys;
pub mod link;
pub mod protocol;
pub mod session;
pub mod shapes;
pub mod shim;
pub mod sources;
pub mod symbols;
pub mod term;

pub use session::{DevOptions, dev};
