use std::any::Any;

use taffy::prelude as tf;
use xui_interface::{
    ComputedStyle, DirtyFlags, Event, EventContext, EventHandlers, EventResult, Key, PaintCommand,
    Rect, Style, TextMeasurer, Widget, WidgetType,
};

use super::{Element, LayoutStyledWidget, computed_layout_style, props_hash};

pub struct StyleScopeWidget {
    pub key: Option<Key>,
    pub children: Vec<Element>,
    pub style: Style,
    pub local_style: Style,
    pub event_handlers: EventHandlers,
}

impl std::fmt::Debug for StyleScopeWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StyleScopeWidget")
            .field("key", &self.key)
            .field("children", &self.children.len())
            .field("style", &self.style)
            .field("local_style", &self.local_style)
            .finish()
    }
}

impl StyleScopeWidget {
    pub fn new(style: Style) -> Self {
        Self {
            key: None,
            children: Vec::new(),
            style,
            local_style: Style::new(),
            event_handlers: EventHandlers::default(),
        }
    }

    pub fn child(mut self, child: impl Into<Element>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn style(mut self, style: Style) -> Self {
        self.local_style = style;
        self
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    event_handler_methods!();
}

impl Widget for StyleScopeWidget {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn node_type(&self) -> WidgetType {
        WidgetType::StyleScope
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn props_hash(&self) -> u64 {
        props_hash(&(&self.style, &self.local_style))
    }

    fn event_handlers_mut(&mut self) -> &mut EventHandlers {
        &mut self.event_handlers
    }

    fn update_from(&mut self, next: &dyn Widget) -> DirtyFlags {
        let Some(next) = next.as_any().downcast_ref::<StyleScopeWidget>() else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };

        if self.style != next.style || self.local_style != next.local_style {
            self.style = next.style.clone();
            self.local_style = next.local_style.clone();
            DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT
        } else {
            DirtyFlags::empty()
        }
    }

    fn style_scope(&self) -> Option<&Style> {
        Some(&self.style)
    }

    fn style(&self) -> &Style {
        &self.local_style
    }

    fn paint(&self, _rect: Rect, _style: &ComputedStyle, _commands: &mut Vec<PaintCommand>) {}

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}

impl LayoutStyledWidget for StyleScopeWidget {
    fn layout_style(
        &self,
        computed: &ComputedStyle,
        _measurer: &mut dyn TextMeasurer,
    ) -> tf::Style {
        computed_layout_style(computed)
    }
}
