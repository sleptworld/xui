use std::any::Any;

use xui_interface::{
    DirtyFlags, Event, EventContext, EventHandlers, EventResult, Key, PaintCommand, Rect, Widget,
    WidgetType,
};

use super::{Element, props_hash};

pub struct ColumnWidget {
    pub key: Option<Key>,
    pub children: Vec<Element>,
    pub gap: f32,
    pub event_handlers: EventHandlers,
}

impl std::fmt::Debug for ColumnWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColumnWidget")
            .field("key", &self.key)
            .field("children", &self.children.len())
            .field("gap", &self.gap)
            .finish()
    }
}

impl ColumnWidget {
    pub fn new() -> Self {
        Self {
            key: None,
            children: Vec::new(),
            gap: 0.0,
            event_handlers: EventHandlers::default(),
        }
    }

    pub fn child(mut self, child: impl Into<Element>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    event_handler_methods!();
}

impl Default for ColumnWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for ColumnWidget {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn node_type(&self) -> WidgetType {
        WidgetType::Column
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn props_hash(&self) -> u64 {
        props_hash(&self.gap.to_bits())
    }

    fn event_handlers_mut(&mut self) -> &mut EventHandlers {
        &mut self.event_handlers
    }

    fn update_from(&mut self, next: &dyn Widget) -> DirtyFlags {
        let Some(next) = next.as_any().downcast_ref::<ColumnWidget>() else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };
        if self.gap != next.gap {
            self.gap = next.gap;
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
