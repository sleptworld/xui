//! The system's state, one context key per field.
//!
//! Each key is read on its own, so a component subscribes to exactly the
//! fields it names:
//!
//! ```ignore
//! use xui::system::{viewport, window};
//!
//! #[component]
//! fn title_bar() {
//!     let maximized: bool = context!(window::Maximized);
//!     let size: Size<f32> = context!(viewport::LogicalSize);
//!     // Reruns its readers only when the width crosses 800, not on every resize.
//!     let wide: bool = memo!(size.width >= 800.0);
//!     view! { .. }
//! }
//! ```
//!
//! The host reports what it knows as it happens; the runtime writes only the
//! fields that changed, and the readers all run afterwards, so every one of
//! them sees the same new state. A field a host never reports keeps its
//! default (see each key); [`crate::platform::PlatformHandle::capabilities`]
//! tells what a host can do.
//!
//! The keys are uninhabited: they name a field, and are never values.

use crate::reactive::ContextKey;

macro_rules! keys {
    ($($(#[$doc:meta])* $name:ident: $value:ty;)*) => {
        $(
            $(#[$doc])*
            #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
            pub enum $name {}

            impl ContextKey for $name {
                type Value = $value;
            }
        )*
    };
}

/// The window's state.
pub mod window {
    use super::ContextKey;

    keys! {
        /// Whether the window fills the screen. `false` until the host says otherwise.
        Fullscreen: bool;
        /// Whether the window is maximized (zoomed, on macOS). `false` until
        /// the host says otherwise.
        Maximized: bool;
        /// Whether the window is minimized. `false` on a host that cannot tell.
        Minimized: bool;
        /// Whether the window has keyboard focus. `true` until the host
        /// reports otherwise, so a host that never does looks active.
        Focused: bool;
        /// Whether the window is on screen: not minimized, occluded or of
        /// zero size. While it is not, animation and timed redraws stop.
        Visible: bool;
    }
}

/// The surface the app draws on.
pub mod viewport {
    use super::ContextKey;
    use crate::core::Size;

    keys! {
        /// The window's content size in logical pixels, which layout uses.
        LogicalSize: Size<f32>;
        /// Physical pixels per logical pixel.
        ScaleFactor: f32;
    }
}
