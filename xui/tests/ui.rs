//! Golden-file tests for the diagnostics the element DSL produces.
//!
//! The DSL deliberately outsources most of its error reporting to rustc: an
//! unknown attribute is a missing method, an unknown tag is a missing function.
//! That is only an improvement over a hand-written attribute table if the
//! resulting messages are actually good, so the messages themselves are pinned
//! here rather than left to chance.
//!
//! Regenerate after a toolchain bump with `TRYBUILD=overwrite cargo test -p xui
//! --test ui`, and read the diff — a message getting worse is a regression.

#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
