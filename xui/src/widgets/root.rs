use std::any::Any;

use taffy::prelude as tf;
use xui_interface::{
    Color, ComputedStyle, DirtyFlags, Event, EventContext, EventHandlers, EventResult,
    FlexDirectionStyle, PaintCommand, Rect, Style, TextMeasurer, Widget, WidgetType,
};

use super::{LayoutStyledWidget, computed_layout_style};

#[derive(Debug)]
pub struct RootWidget {
    event_handlers: EventHandlers,
}

impl Default for RootWidget {
    fn default() -> Self {
        Self {
            event_handlers: EventHandlers::default(),
        }
    }
}

impl Widget for RootWidget {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn node_type(&self) -> WidgetType {
        WidgetType::Container
    }

    fn props_hash(&self) -> u64 {
        0
    }

    fn event_handlers_mut(&mut self) -> &mut EventHandlers {
        &mut self.event_handlers
    }

    fn update_from(&mut self, _next: &dyn Widget) -> DirtyFlags {
        DirtyFlags::empty()
    }

    fn default_style(&self) -> Style {
        Style::new().flex_direction(FlexDirectionStyle::Column)
    }

    fn paint(&self, _rect: Rect, _style: &ComputedStyle, commands: &mut Vec<PaintCommand>) {
        commands.push(PaintCommand::Clear(Color::WHITE));
    }

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}

impl LayoutStyledWidget for RootWidget {
    fn layout_style(
        &self,
        computed: &ComputedStyle,
        _measurer: &mut dyn TextMeasurer,
    ) -> tf::Style {
        computed_layout_style(computed)
    }
}
