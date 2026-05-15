use xui_interface::{
    Color, DirtyFlags, Event, EventContext, EventResult, PaintCommand, Point, PointerButton, Rect,
    Widget, WidgetKind, WidgetType,
};

pub struct RootWidget;

impl Widget for RootWidget {
    fn node_type(&self) -> WidgetType {
        WidgetType::Container
    }

    fn update_from_kind(&mut self, _new_kind: &WidgetKind) -> DirtyFlags {
        DirtyFlags::empty()
    }

    fn paint(&self, _rect: Rect, commands: &mut Vec<PaintCommand>) {
        commands.push(PaintCommand::Clear(Color::WHITE));
    }

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}

pub struct LabelWidget {
    pub text: String,
    pub color: Color,
    pub font_size: f32,
}

impl Widget for LabelWidget {
    fn node_type(&self) -> WidgetType {
        WidgetType::Label
    }

    fn update_from_kind(&mut self, new_kind: &WidgetKind) -> DirtyFlags {
        let WidgetKind::Label {
            text,
            color,
            font_size,
        } = new_kind
        else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };

        let mut flags = DirtyFlags::empty();
        if self.text != *text {
            self.text = text.clone();
            flags |= DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        }
        if self.font_size != *font_size {
            self.font_size = *font_size;
            flags |= DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        }
        if self.color != *color {
            self.color = *color;
            flags |= DirtyFlags::PAINT;
        }
        flags
    }

    fn paint(&self, rect: Rect, commands: &mut Vec<PaintCommand>) {
        commands.push(PaintCommand::Text {
            position: Point::new(rect.x, rect.y + self.font_size),
            text: self.text.clone(),
            color: self.color,
            size: self.font_size,
        });
    }

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}

pub struct ButtonWidget {
    pub text: String,
    pub pressed: bool,
    pub hovered: bool,
    pub on_click: Option<Box<dyn FnMut()>>,
}

impl Widget for ButtonWidget {
    fn node_type(&self) -> WidgetType {
        WidgetType::Button
    }

    fn update_from_kind(&mut self, new_kind: &WidgetKind) -> DirtyFlags {
        let WidgetKind::Button { text, .. } = new_kind else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };

        if self.text != *text {
            self.text = text.clone();
            DirtyFlags::LAYOUT | DirtyFlags::PAINT
        } else {
            DirtyFlags::empty()
        }
    }

    fn paint(&self, rect: Rect, commands: &mut Vec<PaintCommand>) {
        let background = if self.pressed {
            Color::BLUE_500
        } else if self.hovered {
            Color::GRAY_300
        } else {
            Color::GRAY_100
        };
        let text_color = if self.pressed {
            Color::WHITE
        } else {
            Color::BLACK
        };
        commands.push(PaintCommand::FillRect {
            rect,
            color: background,
        });
        commands.push(PaintCommand::StrokeRect {
            rect,
            color: Color::GRAY_300,
            width: 1.0,
        });
        commands.push(PaintCommand::Text {
            position: Point::new(rect.x + 8.0, rect.y + 18.0),
            text: self.text.clone(),
            color: text_color,
            size: 14.0,
        });
    }

    fn handle_event(&mut self, event: &Event, cx: &mut EventContext<'_>) -> EventResult {
        match event {
            Event::PointerMove { .. } => {
                if !self.hovered {
                    self.hovered = true;
                    cx.mark_needs_paint();
                }
                EventResult::Ignored
            }
            Event::PointerDown {
                button: PointerButton::Primary,
                ..
            } => {
                self.pressed = true;
                cx.mark_needs_paint();
                EventResult::Consumed
            }
            Event::PointerUp {
                button: PointerButton::Primary,
                ..
            } => {
                let was_pressed = self.pressed;
                self.pressed = false;
                cx.mark_needs_paint();
                if was_pressed {
                    if let Some(on_click) = self.on_click.as_mut() {
                        on_click();
                    }
                }
                EventResult::Consumed
            }
            _ => EventResult::Ignored,
        }
    }
}

pub struct ColumnWidget {
    pub gap: f32,
}

impl Widget for ColumnWidget {
    fn node_type(&self) -> WidgetType {
        WidgetType::Column
    }

    fn update_from_kind(&mut self, new_kind: &WidgetKind) -> DirtyFlags {
        let WidgetKind::Column { gap } = new_kind else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };
        if self.gap != *gap {
            self.gap = *gap;
            DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT
        } else {
            DirtyFlags::empty()
        }
    }

    fn paint(&self, _rect: Rect, _commands: &mut Vec<PaintCommand>) {}

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}

pub struct RowWidget {
    pub gap: f32,
}

impl Widget for RowWidget {
    fn node_type(&self) -> WidgetType {
        WidgetType::Row
    }

    fn update_from_kind(&mut self, new_kind: &WidgetKind) -> DirtyFlags {
        let WidgetKind::Row { gap } = new_kind else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };
        if self.gap != *gap {
            self.gap = *gap;
            DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT
        } else {
            DirtyFlags::empty()
        }
    }

    fn paint(&self, _rect: Rect, _commands: &mut Vec<PaintCommand>) {}

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}

pub struct ContainerWidget {
    pub background: Color,
}

impl Widget for ContainerWidget {
    fn node_type(&self) -> WidgetType {
        WidgetType::Container
    }

    fn update_from_kind(&mut self, new_kind: &WidgetKind) -> DirtyFlags {
        let WidgetKind::Container { background } = new_kind else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };
        if self.background != *background {
            self.background = *background;
            DirtyFlags::PAINT
        } else {
            DirtyFlags::empty()
        }
    }

    fn paint(&self, rect: Rect, commands: &mut Vec<PaintCommand>) {
        if self.background.a > 0.0 {
            commands.push(PaintCommand::FillRect {
                rect,
                color: self.background,
            });
        }
    }

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}
