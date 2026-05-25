pub use xui_interface::TextMeasurer;

use crate::core::Size;
use xui_interface::{ComputedTextStyle, TextLayoutConstraints};

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
    fn measure_text(&mut self, text: &str, style: &ComputedTextStyle) -> Size {
        self.measure_text_with_constraints(text, style, TextLayoutConstraints::UNBOUNDED)
    }

    fn measure_text_with_constraints(
        &mut self,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Size {
        let font_size = style.font_size;
        let glyph_width = font_size * self.average_glyph_width;
        let natural_width = text.chars().count() as f32 * glyph_width;
        let Some(max_width) = constraints.max_width.filter(|width| *width > 0.0) else {
            return Size::new(natural_width, font_size * self.line_height);
        };
        let chars_per_line = (max_width / glyph_width).floor().max(1.0);
        let line_count = (text.chars().count() as f32 / chars_per_line)
            .ceil()
            .max(1.0);
        Size::new(
            natural_width.min(max_width),
            font_size * self.line_height * line_count,
        )
    }
}
