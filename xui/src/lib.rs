//! Facade over `xui-core` and `xui-macros`: depend on `xui` alone and write
//! `use xui::prelude::*;` / `#[xui::main]`. `xui-core` does not depend on the
//! macros; this crate is where the two meet. The macros fall back to `::xui`
//! when `xui-core` is not a direct dependency, so everything they expand to
//! must stay reachable through the re-exports below.

pub use xui_core::*;
pub use xui_macros::{Animatable, component, component_fn, defaults, main, style, xui};

#[doc(hidden)]
pub use xui_animation;

pub mod prelude {
    pub use xui_core::prelude::*;
    pub use xui_macros::{component, component_fn, defaults, style, xui};
}

#[cfg(target_os = "macos")]
pub use xui_macos::{MacRunnerOptions, runner};
