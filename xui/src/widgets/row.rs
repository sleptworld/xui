use std::any::Any;

use taffy::prelude as tf;
use xui_interface::{
    ComputedStyle, DirtyFlags, Event, EventContext, EventHandlers, EventResult, FlexDirectionStyle,
    Key, PaintCommand, Rect, Style, TextMeasurer, Widget, WidgetType,
};

use super::{Element, LayoutStyledWidget, computed_layout_style, props_hash};

pub struct RowWidget {
    pub key: Option<Key>,
    pub children: Vec<Element>,
    pub style: Style,
    pub event_handlers: EventHandlers,
}

impl std::fmt::Debug for RowWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RowWidget")
            .field("key", &self.key)
            .field("children", &self.children.len())
            .field("style", &self.style)
            .finish()
    }
}

impl RowWidget {
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

impl Default for RowWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for RowWidget {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn node_type(&self) -> WidgetType {
        WidgetType::Row
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
        let Some(next) = next.as_any().downcast_ref::<RowWidget>() else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };
        if self.style != next.style {
            self.style = next.style.clone();
            DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT
        } else {
            DirtyFlags::empty()
        }
    }

    fn default_style(&self) -> Style {
        Style::new().flex_direction(FlexDirectionStyle::Row)
    }

    fn style(&self) -> &Style {
        &self.style
    }

    fn paint(&self, _rect: Rect, _style: &ComputedStyle, _commands: &mut Vec<PaintCommand>) {}

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}

impl LayoutStyledWidget for RowWidget {
    fn layout_style(
        &self,
        computed: &ComputedStyle,
        _measurer: &mut dyn TextMeasurer,
    ) -> tf::Style {
        computed_layout_style(computed)
    }
}
