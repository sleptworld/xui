use xui_interface::{Color, Key, Size, Style, WidgetType, WidgetUpdateFlags, core::Sizing};

use super::props_hash;

#[derive(Debug)]
pub struct RootWidget {
    pub style: Style,
}

impl Default for RootWidget {
    fn default() -> Self {
        Self {
            style: Style::new(),
        }
    }
}

impl RootWidget {
    pub fn set_style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }
}

impl RootWidget {
    pub(super) fn node_type(&self) -> WidgetType {
        WidgetType::Container
    }

    pub(super) fn props_hash(&self) -> u64 {
        props_hash(&self.style)
    }

    pub(super) fn update_from(&mut self, next: &Self) -> WidgetUpdateFlags {
        if self.style != next.style {
            self.style = next.style.clone();
            WidgetUpdateFlags::STYLE_TARGET
        } else {
            WidgetUpdateFlags::empty()
        }
    }

    pub(super) fn default_style(&self) -> Style {
        Style::new()
            .size(Size::<Sizing>::new(
                Sizing::Percent(1.0.try_into().unwrap()),
                Sizing::Percent(1.0.try_into().unwrap()),
            ))
            .background(Color::WHITE)
    }

    pub(super) fn get_key(&self) -> Option<&Key> {
        None
    }

    pub(super) fn current_style(&self) -> &Style {
        &self.style
    }

    no_event_handler_methods!();
}
