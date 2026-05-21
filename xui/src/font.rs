use xui_interface::Size;
use xui_interface::TextMeasurer;
use xui_text::engine::Engine;

pub struct TextI {
    engine: Engine,
}

impl TextI {
    pub fn new() -> Self {
        Self {
            engine: Engine::new(),
        }
    }

    pub fn measure(&mut self, text: &str, max_width: f32) -> Size {
        let mut session =
            self.engine
                .start(xui_text::doc::Direction::Auto, 1.0, max_width as usize);

        session.add_text(text);
        let par = session.finish(None);

        let width = 0.0;
        let mut height = 0.0;

        for line in par.lines() {
            let size = line.size();
            height += size;
        }

        Size::new(width, height)
    }
}

impl TextMeasurer for TextI {
    fn measure(&mut self, text: &str, font_size: f32) -> Size {
        TextI::measure(self, text, font_size)
    }
}
