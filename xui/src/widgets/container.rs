use crate::element::ElementDesc;
use crate::event_system::EventContext;
use crate::event_system::callbacks::EventHandlers;
use xui_animation::Transition;
use xui_interface::style::FlexDirectionStyle;
use xui_interface::{
    ColorStyle, ComputedStyle, EventRef, EventResult, Key, LengthValue, PaintCommand, Rect,
    ScrollDirectionStyle, Style, TextContent, TextProps, WidgetType, WidgetUpdateFlags,
    style::ScrollbarStylePatch,
};

use super::{props_hash, widget_element_desc};

pub struct ContainerWidget {
    pub key: Option<Key>,
    pub style: Style,
    pub flex_direction: Option<FlexDirectionStyle>,
    pub transition: Option<Transition>,
    pub event_handlers: EventHandlers,
}

impl std::fmt::Debug for ContainerWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContainerWidget")
            .field("key", &self.key)
            .field("flex_direction", &self.flex_direction)
            .field("transition", &self.transition)
            .finish()
    }
}

impl ContainerWidget {
    pub fn new() -> Self {
        Self {
            key: None,
            style: Style::default(),
            flex_direction: None,
            transition: None,
            event_handlers: EventHandlers::default(),
        }
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn transition(mut self, transition: Transition) -> Self {
        self.transition = Some(transition);
        self
    }

    pub fn flex_direction(mut self, direction: FlexDirectionStyle) -> Self {
        self.flex_direction = Some(direction);
        self
    }

    pub fn scrollable(mut self) -> Self {
        let style = self.style.scroll_vertical();
        self.style = style;
        self
    }

    pub fn scroll_direction(mut self, direction: ScrollDirectionStyle) -> Self {
        let style = self.style.scroll_direction(direction);
        self.style = style;
        self
    }

    pub fn scrollbar(mut self, scrollbar: ScrollbarStylePatch) -> Self {
        let style = self.style.scrollbar(scrollbar);
        self.style = style;
        self
    }

    pub fn scrollbar_width(mut self, width: impl Into<LengthValue>) -> Self {
        let style = self.style.scrollbar_width(width);
        self.style = style;
        self
    }

    pub fn scrollbar_track_color(mut self, color: impl Into<ColorStyle>) -> Self {
        let style = self.style.scrollbar_track_color(color);
        self.style = style;
        self
    }

    pub fn scrollbar_thumb_color(mut self, color: impl Into<ColorStyle>) -> Self {
        let style = self.style.scrollbar_thumb_color(color);
        self.style = style;
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

impl ContainerWidget {
    pub(super) fn node_type(&self) -> WidgetType {
        WidgetType::Container
    }

    pub(super) fn get_key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    pub(super) fn props_hash(&self) -> u64 {
        props_hash(&(&self.style, self.flex_direction, self.transition))
    }

    pub(super) fn update_from(&mut self, next: &Self) -> WidgetUpdateFlags {
        let mut flags = WidgetUpdateFlags::empty();
        if self.style != next.style {
            self.style = next.style.clone();
            flags |= WidgetUpdateFlags::STYLE_TARGET;
        }
        if self.flex_direction != next.flex_direction {
            self.flex_direction = next.flex_direction;
            flags |= WidgetUpdateFlags::LAYOUT_INPUT;
        }
        if self.transition != next.transition {
            self.transition = next.transition;
            flags |= WidgetUpdateFlags::STYLE_TARGET;
        }

        if flags.is_empty() {
            WidgetUpdateFlags::empty()
        } else {
            flags
        }
    }

    pub(super) fn default_style(&self) -> Style {
        Style::new()
    }

    pub(super) fn current_style(&self) -> &Style {
        &self.style
    }

    pub(super) fn paint(
        &self,
        rect: Rect,
        style: &ComputedStyle,
        commands: &mut Vec<PaintCommand>,
    ) {
        paint_box(rect, style, commands);
    }

    pub(super) fn handle_event(
        &mut self,
        _event: EventRef<'_>,
        _cx: &mut EventContext<'_>,
    ) -> EventResult {
        EventResult::Ignored
    }

    pub(super) fn text_content(&self) -> Option<TextContent> {
        None
    }

    pub(super) fn text_layout_props(&self, _style: &ComputedStyle) -> Option<TextProps> {
        None
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
