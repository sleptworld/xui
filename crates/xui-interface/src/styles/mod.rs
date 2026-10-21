//! Style value vocabulary.
//!
//! [`units`] are the declared value types, [`computed`] what they resolve to,
//! and [`compute`] how one becomes the other against a theme. Which property a
//! node has, and where its declaration lands in a `ComputedStyle`, is the
//! business of the style capabilities in `xui-core`.

pub mod compute;
pub mod computed;
pub mod units;
