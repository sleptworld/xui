pub use xui_interface::TextMeasurer;

use crate::core::Size;

#[derive(Debug, Clone, Copy)]
pub struct MockTextMeasurer {
    pub average_glyph_width: f32,
    pub line_height: f32,
}

impl Default for MockTextMeasurer {
    fn default() -> Self {
        Self {
            average_glyph_width: 0.58,
            line_height: 1.25,
        }
    }
}

impl TextMeasurer for MockTextMeasurer {
    fn measure(&mut self, text: &str, font_size: f32) -> Size {
        Size::new(
            text.chars().count() as f32 * font_size * self.average_glyph_width,
            font_size * self.line_height,
        )
    }
}
