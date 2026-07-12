use crate::Rect;

/// Platform-facing state desired by the UI runtime.
///
/// Backends should compare this value with the last state they applied and
/// translate only the differences to their native windowing API.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlatformOutput {
    pub text_input: Option<TextInputSession>,
}

/// The active native text-input/IME session.
#[derive(Debug, Clone, PartialEq)]
pub struct TextInputSession {
    /// Candidate-window anchor in logical coordinates relative to the window.
    pub cursor_area: Rect,
    pub purpose: TextInputPurpose,
    pub multiline: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextInputPurpose {
    #[default]
    Normal,
    Password,
    Email,
    Number,
    Phone,
    Url,
}
