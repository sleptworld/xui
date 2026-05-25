use std::sync::Arc;
use xui_interface::{Size, TextLayoutConstraints, TextMeasurer, TextProps};
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
        self.engine.measure(text, font_size)
    }
}

impl TextMeasurer for TextI {
    fn measure_text(&mut self, props: &TextProps) -> Size {
        self.engine.measure_text(props)
    }

    fn measure_text_with_constraints(
        &mut self,
        props: &TextProps,
        constraints: TextLayoutConstraints,
    ) -> Size {
        self.engine
            .measure_text_with_constraints(props, constraints)
    }
}

impl TextLayouter for TextI {
    fn layout_text(&mut self, props: &TextProps, constraints: TextLayoutConstraints) -> Arc<Par> {
        self.engine.layout_text(props, constraints)
    }
}
