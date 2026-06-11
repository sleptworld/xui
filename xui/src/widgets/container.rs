use crate::element::ElementDesc;
use xui_interface::{
    ColorStyle, ComputedStyle, DirtyFlags, Event, EventContext, EventHandlers, EventResult, Key,
    LengthValue, PaintCommand, Rect, ScrollDirectionStyle, ScrollbarStyle,
    ScrollbarVisibilityStyle, Style, Widget, WidgetType, style::ScrollbarStylePatch,
};

use super::{props_hash, widget_element_desc};

pub struct ContainerWidget {
    pub key: Option<Key>,
    pub style: Style,
    pub event_handlers: EventHandlers,
}

impl std::fmt::Debug for ContainerWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContainerWidget")
            .field("key", &self.key)
            .field("style", &self.style)
            .finish()
    }
}

impl ContainerWidget {
    pub fn new() -> Self {
        Self {
            key: None,
            style: Style::new(),
            event_handlers: EventHandlers::default(),
        }
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn scrollable(mut self) -> Self {
        self.style = self.style.clone().scroll_vertical();
        self
    }

    pub fn scroll_direction(mut self, direction: ScrollDirectionStyle) -> Self {
        self.style = self.style.clone().scroll_direction(direction);
        self
    }

    pub fn scrollbar(mut self, scrollbar: ScrollbarStylePatch) -> Self {
        self.style = self.style.clone().scrollbar(scrollbar);
        self
    }

    pub fn scrollbar_width(mut self, width: impl Into<LengthValue>) -> Self {
        self.style = self.style.clone().scrollbar_width(width);
        self
    }

    pub fn scrollbar_track_color(mut self, color: impl Into<ColorStyle>) -> Self {
        self.style = self.style.clone().scrollbar_track_color(color);
        self
    }

    pub fn scrollbar_thumb_color(mut self, color: impl Into<ColorStyle>) -> Self {
        self.style = self.style.clone().scrollbar_thumb_color(color);
        self
    }

    pub fn scrollbar_radius(mut self, radius: impl Into<LengthValue>) -> Self {
        self.style = self.style.clone().scrollbar_radius(radius);
        self
    }

    pub fn scrollbar_visibility(mut self, visibility: ScrollbarVisibilityStyle) -> Self {
        self.style = self.style.clone().scrollbar_visibility(visibility);
        self
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn into_element_desc(self, children: Vec<ElementDesc>) -> ElementDesc {
        widget_element_desc(self, children)
    }

    event_handler_methods!();
}

impl Default for ContainerWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for ContainerWidget {
    fn node_type(&self) -> WidgetType {
        WidgetType::Container
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn props_hash(&self) -> u64 {
        props_hash(&self.style)
    }

    fn update_from(&mut self, next: &Self) -> DirtyFlags {
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

pub(super) fn paint_box(rect: Rect, style: &ComputedStyle, commands: &mut Vec<PaintCommand>) {
    let paint = style.paint;

    let cmd = if paint.border_radius > 0.0 {
        PaintCommand::RoundedRect {
            rect,
            radius: paint.border_radius,
            color: paint.background,
            stroke: paint.stroke,
            shadow: paint.shadow,
        }
    } else {
        PaintCommand::Rect {
            rect,
            color: paint.background,
            stroke: paint.stroke,
            shadow: paint.shadow,
        }
    };

    commands.push(cmd);
}
