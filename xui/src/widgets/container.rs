use std::any::Any;

use xui_interface::{
    DirtyFlags, EdgeInsets, Event, EventContext, EventHandlers, EventResult, Key, PaintCommand,
    Rect, Size, Widget, WidgetType,
};

use crate::core::Color;

use super::{Element, hash_color, hash_edge_insets};

pub struct ContainerWidget {
    pub key: Option<Key>,
    pub children: Vec<Element>,
    pub size: Option<Size>,
    pub padding: EdgeInsets,
    pub background: Color,
    pub event_handlers: EventHandlers,
}

impl std::fmt::Debug for ContainerWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContainerWidget")
            .field("key", &self.key)
            .field("children", &self.children.len())
            .field("size", &self.size)
            .field("padding", &self.padding)
            .field("background", &self.background)
            .finish()
    }
}

impl ContainerWidget {
    pub fn new() -> Self {
        Self {
            key: None,
            children: Vec::new(),
            size: None,
            padding: EdgeInsets::ZERO,
            background: Color::TRANSPARENT,
            event_handlers: EventHandlers::default(),
        }
    }

    pub fn child(mut self, child: impl Into<Element>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn size(mut self, size: Size) -> Self {
        self.size = Some(size);
        self
    }

    pub fn padding(mut self, padding: EdgeInsets) -> Self {
        self.padding = padding;
        self
    }

    pub fn background(mut self, background: Color) -> Self {
        self.background = background;
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
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(
            &self
                .size
                .map(|size| (size.width.to_bits(), size.height.to_bits())),
            &mut hasher,
        );
        hash_edge_insets(self.padding, &mut hasher);
        hash_color(self.background, &mut hasher);
        std::hash::Hasher::finish(&hasher)
    }

    fn event_handlers_mut(&mut self) -> &mut EventHandlers {
        &mut self.event_handlers
    }

    fn update_from(&mut self, next: &dyn Widget) -> DirtyFlags {
        let Some(next) = next.as_any().downcast_ref::<ContainerWidget>() else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };

        let mut flags = DirtyFlags::empty();
        if self.size != next.size {
            self.size = next.size;
            flags |= DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        }
        if self.padding != next.padding {
            self.padding = next.padding;
            flags |= DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        }
        if self.background != next.background {
            self.background = next.background;
            flags |= DirtyFlags::PAINT;
        }

        if flags.is_empty() {
            DirtyFlags::empty()
        } else {
            flags
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
