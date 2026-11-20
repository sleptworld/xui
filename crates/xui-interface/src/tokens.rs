//! The built-in design tokens, by kind.
//!
//! Their fallbacks are the light theme, shadcn/ui's neutral palette, so
//! [`Theme::light`](crate::style::Theme::light) sets nothing and
//! [`Theme::dark`](crate::style::Theme::dark) sets every color.
//!
//! ```ignore
//! use xui::tokens::{color, radius, spacing};
//! container().background(color::CARD).padding(spacing::MD).radius(radius::SM)
//! ```

/// Color roles. A surface's `*_FOREGROUND` is the text drawn on it. Text has
/// four levels: [`FOREGROUND_STRONG`](color::FOREGROUND_STRONG) >
/// [`FOREGROUND`](color::FOREGROUND) > [`MUTED_FOREGROUND`](color::MUTED_FOREGROUND) >
/// [`FOREGROUND_FAINT`](color::FOREGROUND_FAINT).
pub mod color {
    use crate::{Color, Token};

    const fn token(name: &'static str, rgb: u32) -> Token<Color> {
        Token::new(name, Color::from_rgb_u32(rgb))
    }

    /// Window ground.
    pub const BACKGROUND: Token<Color> = token("background", 0xffffff);
    /// Headings and emphasized text.
    pub const FOREGROUND_STRONG: Token<Color> = token("foreground.strong", 0x0a0a0a);
    /// Body text.
    pub const FOREGROUND: Token<Color> = token("foreground", 0x0a0a0a);
    /// Secondary text: descriptions, placeholders, inactive tabs.
    pub const MUTED_FOREGROUND: Token<Color> = token("muted.foreground", 0x737373);
    /// The weakest text: disabled labels, separators.
    pub const FOREGROUND_FAINT: Token<Color> = token("foreground.faint", 0xa1a1a1);
    /// Cards and panels.
    pub const CARD: Token<Color> = token("card", 0xffffff);
    pub const CARD_FOREGROUND: Token<Color> = token("card.foreground", 0x0a0a0a);
    /// Menus, dialogs and other floating layers.
    pub const POPOVER: Token<Color> = token("popover", 0xffffff);
    pub const POPOVER_FOREGROUND: Token<Color> = token("popover.foreground", 0x0a0a0a);
    /// Primary actions.
    pub const PRIMARY: Token<Color> = token("primary", 0x171717);
    pub const PRIMARY_FOREGROUND: Token<Color> = token("primary.foreground", 0xfafafa);
    /// Secondary actions.
    pub const SECONDARY: Token<Color> = token("secondary", 0xf5f5f5);
    pub const SECONDARY_FOREGROUND: Token<Color> = token("secondary.foreground", 0x171717);
    /// Subdued surfaces such as a tab list or skeleton.
    pub const MUTED: Token<Color> = token("muted", 0xf5f5f5);
    /// Hover and highlight surface for menu items and ghost buttons.
    pub const ACCENT: Token<Color> = token("accent", 0xf5f5f5);
    pub const ACCENT_FOREGROUND: Token<Color> = token("accent.foreground", 0x171717);
    /// Destructive actions.
    pub const DESTRUCTIVE: Token<Color> = token("destructive", 0xe7000b);
    pub const DESTRUCTIVE_FOREGROUND: Token<Color> = token("destructive.foreground", 0xffffff);
    /// Selected rows and text selections.
    pub const SELECTION: Token<Color> = token("selection", 0xe5e5e5);
    pub const SELECTION_FOREGROUND: Token<Color> = token("selection.foreground", 0x0a0a0a);
    /// Dividers and borders.
    pub const BORDER: Token<Color> = token("border", 0xe5e5e5);
    /// Border of form controls.
    pub const INPUT: Token<Color> = token("input", 0xe5e5e5);
    /// Focus ring.
    pub const RING: Token<Color> = token("ring", 0xa1a1a1);
}

/// Gaps and padding.
pub mod spacing {
    use crate::Token;

    pub const XS: Token<f32> = Token::new("spacing.xs", 4.0);
    pub const SM: Token<f32> = Token::new("spacing.sm", 8.0);
    pub const MD: Token<f32> = Token::new("spacing.md", 12.0);
    pub const LG: Token<f32> = Token::new("spacing.lg", 16.0);
    pub const XL: Token<f32> = Token::new("spacing.xl", 24.0);
}

/// Corner radii, derived from a 10px base radius as in shadcn/ui.
pub mod radius {
    use crate::Token;

    pub const SM: Token<f32> = Token::new("radius.sm", 6.0);
    pub const MD: Token<f32> = Token::new("radius.md", 8.0);
    pub const LG: Token<f32> = Token::new("radius.lg", 10.0);
    pub const XL: Token<f32> = Token::new("radius.xl", 14.0);
}

/// Text sizes.
pub mod font_size {
    use crate::Token;

    pub const SM: Token<f32> = Token::new("font-size.sm", 12.0);
    pub const MD: Token<f32> = Token::new("font-size.md", 14.0);
    pub const LG: Token<f32> = Token::new("font-size.lg", 16.0);
    pub const XL: Token<f32> = Token::new("font-size.xl", 20.0);
}
