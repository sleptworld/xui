//! What `cargo xui dev` and the app say to each other.
//!
//! The app (`xui-core`, feature `hotpatch`) connects over TCP to the address
//! in `XUI_HOT_ADDR`. Each message is one line of JSON, an object with one
//! key naming it. `xui-core` reads and writes these shapes without
//! depending on this crate; change both together.
//!
//! ```text
//! app → dev   {"hello":   {"pid": 123, "aslr_reference": 4295000000, "build": "…"}}
//! dev → app   {"patch":   {"id": 1, "table": <subsecond JumpTable>, "remount": ["greeting"], "mirror": {"address": 4295000064, "len": 16384}}}
//! app → dev   {"applied": {"id": 1, "error": null}}
//! app → dev   {"reloaded":{"changed": 1, "called": 1, "renamed": 0, "remounted": 1, "kept": 6, "micros": 1800, "full": false}}
//! ```
//!
//! `remount` names components (by function) whose code is the same but
//! calls code the patch changed: a prop that reads no signal, and a
//! handler, keep what they were built with, so these are mounted again.
//! `called` counts them in `changed`.
//!
//! ```text
//! ```
//!
//! `aslr_reference` is where the app's `main` is loaded: patches are linked
//! against the app's addresses, so a patch is only good for the process
//! that said hello. `build` echoes `XUI_HOT_BUILD`, so a process from an
//! older build is told apart.

use serde::{Deserialize, Serialize};

/// The address the app connects to.
pub const ADDR_ENV: &str = "XUI_HOT_ADDR";
/// The build the app was launched from, which it says back in its hello.
pub const BUILD_ENV: &str = "XUI_HOT_BUILD";

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FromApp {
    Hello {
        pid: u32,
        aslr_reference: u64,
        build: String,
    },
    /// A patch was loaded, or why not.
    Applied { id: u64, error: Option<String> },
    /// The runtime mounted again what the patch changed.
    Reloaded {
        changed: usize,
        /// Of `changed`, mounted again for code they call.
        #[serde(default)]
        called: usize,
        renamed: usize,
        remounted: usize,
        kept: usize,
        micros: u64,
        /// Everything was mounted again (a root changed).
        full: bool,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToApp {
    Patch {
        id: u64,
        table: subsecond_types::JumpTable,
        /// Components to mount again for code they call, by function name.
        remount: Vec<String>,
        /// The app's pages to map over the patch's mirror before it runs.
        mirror: Option<Mirror>,
    },
}

/// The app's pages that hold the binary crate's statics, which a patch
/// shares: the app maps them, shared, over the zero-fill section the
/// patch exports as `xui_hot_mirror`, before any of the patch's code runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mirror {
    /// In the running app (slid), page-aligned.
    pub address: u64,
    /// A whole number of pages.
    pub len: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lines `xui-core`'s `hot.rs` writes, which it builds by hand.
    #[test]
    fn reads_what_the_app_writes() {
        let hello: FromApp =
            serde_json::from_str(r#"{"hello":{"pid":7,"aslr_reference":4294983680,"build":"ab"}}"#)
                .unwrap();
        assert!(
            matches!(hello, FromApp::Hello { pid: 7, aslr_reference: 4294983680, ref build } if build == "ab")
        );
        let applied: FromApp =
            serde_json::from_str(r#"{"applied":{"id":3,"error":null}}"#).unwrap();
        assert!(matches!(applied, FromApp::Applied { id: 3, error: None }));
        let reloaded: FromApp = serde_json::from_str(
            r#"{"reloaded":{"changed":1,"called":1,"renamed":0,"remounted":1,"kept":6,"micros":1800,"full":false}}"#,
        )
        .unwrap();
        assert!(matches!(
            reloaded,
            FromApp::Reloaded {
                called: 1,
                kept: 6,
                full: false,
                ..
            }
        ));
    }

    /// What the app reads: `patch.id`, `patch.remount`, and `patch.table`
    /// as subsecond's table.
    #[test]
    fn writes_what_the_app_reads() {
        let mut map = subsecond_types::AddressMap::default();
        map.insert(0x1000, 0x2000);
        let line = serde_json::to_value(ToApp::Patch {
            id: 4,
            table: subsecond_types::JumpTable {
                lib: "/tmp/libpatch-4.dylib".into(),
                map,
                aslr_reference: 1,
                new_base_address: 2,
                ifunc_count: 0,
            },
            remount: vec!["greeting".into()],
            mirror: Some(Mirror {
                address: 0x10_0000_4000,
                len: 0x8000,
            }),
        })
        .unwrap();
        assert_eq!(line["patch"]["id"], 4);
        assert_eq!(line["patch"]["remount"][0], "greeting");
        assert_eq!(line["patch"]["mirror"]["address"], 0x10_0000_4000_u64);
        assert_eq!(line["patch"]["mirror"]["len"], 0x8000);
        let table: subsecond_types::JumpTable =
            serde_json::from_value(line["patch"]["table"].clone()).unwrap();
        assert_eq!(table.map.get(&0x1000), Some(&0x2000));
    }
}
