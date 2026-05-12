use log::info;
use xui::PaintCommand;
use xui_interface::core::*;

pub struct CmdHandler {
    clip_stack: Vec<Rect>,
    transform_stack: Vec<Point>,
}

impl CmdHandler {
    pub fn handle_command(&mut self, cmd: &PaintCommand) {
        match cmd {
            PaintCommand::FillRect { rect, color } => {
                self.fill_rect(rect, color);
            }
            PaintCommand::StrokeRect { rect, color, width } => {
                self.stroke_rect(rect, color, *width);
            }
            PaintCommand::FillRoundedRect {
                rect,
                radius,
                color,
            } => {
                self.fill_rounded_rect(rect, *radius, color);
            }
            PaintCommand::StrokeRoundedRect {
                rect,
                radius,
                color,
                width,
            } => {
                self.stroke_rounded_rect(rect, *radius, color, *width);
            }
            PaintCommand::Line {
                from,
                to,
                color,
                width,
            } => {
                self.line(from, to, color, *width);
            }
            PaintCommand::Text {
                position,
                text,
                color,
                size,
            } => {
                self.text(position, text, color, *size);
            }
            PaintCommand::PushClip(rect) => {
                self.push_clip(rect);
            }
            PaintCommand::PopClip => {
                self.pop_clip();
            }
            PaintCommand::PushTransform { translate } => {
                self.push_transform(translate);
            }
            PaintCommand::PopTransform => {
                self.pop_transform();
            }

            PaintCommand::Clear(color) => {}
        }
    }

    fn fill_rect(&mut self, rect: &Rect, color: &Color) {
        info!("FillRect: rect={:?}, color={:?}", rect, color);
    }

    fn stroke_rect(&mut self, rect: &Rect, color: &Color, width: f32) {
        info!(
            "StrokeRect: rect={:?}, color={:?}, width={}",
            rect, color, width
        );
    }

    fn fill_rounded_rect(&mut self, rect: &Rect, radius: f32, color: &Color) {
        info!(
            "FillRoundedRect: rect={:?}, radius={}, color={:?}",
            rect, radius, color
        );
    }

    fn stroke_rounded_rect(&mut self, rect: &Rect, radius: f32, color: &Color, width: f32) {
        info!(
            "StrokeRoundedRect: rect={:?}, radius={}, color={:?}, width={}",
            rect, radius, color, width
        );
    }

    fn line(&mut self, from: &Point, to: &Point, color: &Color, width: f32) {
        info!(
            "Line: from={:?}, to={:?}, color={:?}, width={}",
            from, to, color, width
        );
    }

    fn text(&mut self, position: &Point, text: &str, color: &Color, size: f32) {
        info!(
            "Text: position={:?}, text=\"{}\", color={:?}, size={}",
            position, text, color, size
        );
    }

    fn push_clip(&mut self, rect: &Rect) {
        info!("PushClip: rect={:?}", rect);
        self.clip_stack.push(*rect);
    }

    fn pop_clip(&mut self) {
        info!("PopClip");
        self.clip_stack.pop();
    }

    fn push_transform(&mut self, translate: &Point) {
        info!("PushTransform: translate={:?}", translate);
        self.transform_stack.push(*translate);
    }

    fn pop_transform(&mut self) {
        info!("PopTransform");
        self.transform_stack.pop();
    }
}
