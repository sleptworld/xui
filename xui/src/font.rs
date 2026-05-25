use std::sync::Arc;
use xui_interface::{ComputedTextStyle, Size, TextLayoutConstraints, TextMeasurer, TextStyle};
use xui_text::engine::{Engine, TextLayouter};
use xui_text::par::Par;

pub struct TextI {
    engine: Engine,
}

impl TextI {
    pub fn new() -> Self {
        Self {
            engine: Engine::new(),
        }
    }

    pub fn measure(&mut self, text: &str, font_size: f32) -> Size {
        let style = ComputedTextStyle {
            font_size,
            ..TextStyle::default().into()
        };
        self.engine.measure_text(text, &style)
    }
}

impl TextMeasurer for TextI {
    fn measure_text(&mut self, text: &str, style: &ComputedTextStyle) -> Size {
        self.engine.measure_text(text, style)
    }

    fn measure_text_with_constraints(
        &mut self,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Size {
        self.engine
            .measure_text_with_constraints(text, style, constraints)
    }
}

impl TextLayouter for TextI {
    fn layout_text(
        &mut self,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Arc<Par> {
        self.engine.layout_text(text, style, constraints)
    }
}
