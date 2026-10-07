//! What the host knows about itself and its surroundings.

use crate::Color;

/// What a host can do. Reported once at startup, so components can hide or
/// degrade UI instead of offering something that silently does nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct HostCapabilities(u32);

impl HostCapabilities {
    pub const NONE: Self = Self(0);
    pub const CLIPBOARD: Self = Self(1 << 0);
    /// X11 / Wayland primary selection (middle-click paste).
    pub const SELECTION_CLIPBOARD: Self = Self(1 << 1);
    pub const FULLSCREEN: Self = Self(1 << 2);
    pub const WINDOW_DRAG: Self = Self(1 << 3);
    pub const DRAG_DROP: Self = Self(1 << 4);
    pub const FILE_DIALOGS: Self = Self(1 << 5);
    pub const MESSAGE_BOX: Self = Self(1 << 6);
    pub const NATIVE_MENU_BAR: Self = Self(1 << 7);
    pub const NATIVE_CONTEXT_MENU: Self = Self(1 << 8);
    pub const TOUCH: Self = Self(1 << 9);
    pub const GESTURES: Self = Self(1 << 10);
    pub const OPEN_URL: Self = Self(1 << 11);
    pub const NOTIFICATIONS: Self = Self(1 << 12);
    pub const SYSTEM_APPEARANCE: Self = Self(1 << 13);

    /// Whether every capability in `other` is present.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for HostCapabilities {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for HostCapabilities {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ColorScheme {
    #[default]
    Light,
    Dark,
}

/// The system's appearance preferences.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Appearance {
    pub color_scheme: ColorScheme,
    pub reduce_motion: bool,
    pub reduce_transparency: bool,
    pub high_contrast: bool,
    pub accent: Option<Color>,
}

/// The window states the user can change without going through the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowState {
    pub fullscreen: bool,
    pub maximized: bool,
}

/// Where the window is, for saving and restoring. Physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowGeometry {
    pub outer_position: (i32, i32),
    pub inner_size: (u32, u32),
    pub state: WindowState,
}
