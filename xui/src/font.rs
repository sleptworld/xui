use xui_interface::Size;
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

        let font_size = max_width.max(1.0);
        let width = text.chars().count() as f32 * font_size * 0.58;
        let mut height = 0.0;

        for line in par.lines() {
            let size = line.size();
            height += size;
        }

        if height <= 0.0 {
            height = font_size * 1.25;
        }

        Size::new(width, height)
    }
}
