//! XUI Property / Capability / Typed Slot reference design, v3.
//! All derive/macro expansion is handwritten. No third-party dependencies.
//!
//! Unselected capabilities are rejected at compile time:
//!
//! ```compile_fail
//! use xui_property_architecture::interaction::{ClickHandler, OnClick};
//! use xui_property_architecture::nodes::{Image, NodeDraft};
//! let mut image = NodeDraft::<Image>::image("sample.png");
//! image.set::<OnClick>(Some(ClickHandler::new(|| {})));
//! ```
//!
//! ```compile_fail
//! use xui_property_architecture::interaction::TabIndex;
//! use xui_property_architecture::nodes::{Image, NodeDraft};
//! let mut image = NodeDraft::<Image>::image("sample.png");
//! image.set::<TabIndex>(2);
//! ```

//! ```compile_fail
//! use xui_property_architecture::caps::ButtonStylePatch;
//! let mut patch = ButtonStylePatch::default();
//! patch.interaction.on_click = None; // interaction is absent from state-style patches
//! ```

//! ```compile_fail
//! use xui_property_architecture::managed::GlobalShortcut;
//! use xui_property_architecture::nodes::{Image, NodeDraft};
//! let mut image = NodeDraft::<Image>::image("a.png");
//! image.set::<GlobalShortcut>(None); // Image has no ShortcutAbility
//! ```

pub mod abilities;
pub mod capability;
pub mod caps;
pub mod core;
pub mod interaction;
pub mod managed;
pub mod nodes;
pub mod property;
pub mod runtime;
pub mod template;
pub mod widget;
