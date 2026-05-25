use std::any::Any;

use taffy::prelude as tf;
use xui_interface::{
    ComputedStyle, DirtyFlags, Event, EventContext, EventHandlers, EventResult, Key, PaintCommand,
    Rect, Style, TextMeasurer, Widget, WidgetType,
};

use super::{Element, LayoutStyledWidget, computed_layout_style, props_hash};

pub struct ContainerWidget {
    pub key: Option<Key>,
    pub children: Vec<Element>,
    pub style: Style,
    pub event_handlers: EventHandlers,
}

impl std::fmt::Debug for ContainerWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContainerWidget")
            .field("key", &self.key)
            .field("children", &self.children.len())
            .field("style", &self.style)
            .finish()
    }
}

impl ContainerWidget {
    pub fn new() -> Self {
        Self {
            key: None,
            children: Vec::new(),
            style: Style::new(),
            event_handlers: EventHandlers::default(),
        }
    }

    pub fn child(mut self, child: impl Into<Element>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    event_handler_methods!();
}

impl Default for ContainerWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for ContainerWidget {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn node_type(&self) -> WidgetType {
        WidgetType::Container
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn props_hash(&self) -> u64 {
        props_hash(&self.style)
    }

    fn event_handlers_mut(&mut self) -> &mut EventHandlers {
        &mut self.event_handlers
    }

    fn update_from(&mut self, next: &dyn Widget) -> DirtyFlags {
        let Some(next) = next.as_any().downcast_ref::<ContainerWidget>() else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };

        let mut flags = DirtyFlags::empty();
        if self.style != next.style {
            self.style = next.style.clone();
            flags |= DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        }

        if flags.is_empty() {
            DirtyFlags::empty()
        } else {
            flags
        }
    }

    fn default_style(&self) -> Style {
        Style::new()
    }

    fn style(&self) -> &Style {
        &self.style
    }

    fn paint(&self, rect: Rect, style: &ComputedStyle, commands: &mut Vec<PaintCommand>) {
        paint_box(rect, style, commands);
    }

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}

impl LayoutStyledWidget for ContainerWidget {
    fn layout_style(
        &self,
        computed: &ComputedStyle,
        _measurer: &mut dyn TextMeasurer,
    ) -> tf::Style {
        computed_layout_style(computed)
    }
}

pub(super) fn paint_box(rect: Rect, style: &ComputedStyle, commands: &mut Vec<PaintCommand>) {
    let paint = style.paint;
    if paint.background.a > 0.0 {
        if paint.border_radius > 0.0 {
            commands.push(PaintCommand::FillRoundedRect {
                rect,
                radius: paint.border_radius,
                color: paint.background,
            });
        } else {
            commands.push(PaintCommand::FillRect {
                rect,
                color: paint.background,
            });
        }
    }

    if paint.border_width > 0.0 && paint.border_color.a > 0.0 {
        if paint.border_radius > 0.0 {
            commands.push(PaintCommand::StrokeRoundedRect {
                rect,
                radius: paint.border_radius,
                color: paint.border_color,
                width: paint.border_width,
            });
        } else {
            commands.push(PaintCommand::StrokeRect {
                rect,
                color: paint.border_color,
                width: paint.border_width,
            });
        }
    }
}
