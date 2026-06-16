use crate::animation::AnimatedStyle;
use crate::element::ElementDesc;
use crate::event_system::callbacks::EventHandlers;
use xui_interface::{
    ColorStyle, ComputedStyle, DirtyFlags, EventContext, EventRef, EventResult, ImageKey,
    ImagePaintCommand, Key, LengthValue, PaintCommand, Rect, ScrollDirectionStyle,
    ScrollbarVisibilityStyle, Style, Widget, WidgetType, style::ScrollbarStylePatch,
};

use super::{props_hash, widget_element_desc};

pub struct ImageWidget {
    pub key: Option<Key>,
    pub image_key: ImageKey,
    pub opacity: f32,
    pub animated_style: AnimatedStyle,
    pub event_handlers: EventHandlers,
}

impl std::fmt::Debug for ImageWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageWidget")
            .field("key", &self.key)
            .field("image_key", &self.image_key)
            .field("opacity", &self.opacity)
            .field("animated_style", &self.animated_style)
            .finish()
    }
}

impl ImageWidget {
    pub fn new() -> Self {
        Self {
            key: None,
            image_key: "".into(),
            opacity: 1.0,
            animated_style: AnimatedStyle::new(Style::new()),
            event_handlers: EventHandlers::default(),
        }
    }

    pub fn with_image_key(image_key: impl Into<ImageKey>) -> Self {
        Self::new().image_key(image_key)
    }

    pub fn style(mut self, style: Style) -> Self {
        self.animated_style.base = style;
        self
    }

    animated_style_methods!(animated_style);

    pub fn scrollable(mut self) -> Self {
        self.animated_style.base = self.animated_style.base.clone().scroll_vertical();
        self
    }

    pub fn scroll_direction(mut self, direction: ScrollDirectionStyle) -> Self {
        self.animated_style.base = self.animated_style.base.clone().scroll_direction(direction);
        self
    }

    pub fn scrollbar(mut self, scrollbar: ScrollbarStylePatch) -> Self {
        self.animated_style.base = self.animated_style.base.clone().scrollbar(scrollbar);
        self
    }

    pub fn scrollbar_width(mut self, width: impl Into<LengthValue>) -> Self {
        self.animated_style.base = self.animated_style.base.clone().scrollbar_width(width);
        self
    }

    pub fn scrollbar_track_color(mut self, color: impl Into<ColorStyle>) -> Self {
        self.animated_style.base = self
            .animated_style
            .base
            .clone()
            .scrollbar_track_color(color);
        self
    }

    pub fn scrollbar_thumb_color(mut self, color: impl Into<ColorStyle>) -> Self {
        self.animated_style.base = self
            .animated_style
            .base
            .clone()
            .scrollbar_thumb_color(color);
        self
    }

    pub fn scrollbar_radius(mut self, radius: impl Into<LengthValue>) -> Self {
        self.animated_style.base = self.animated_style.base.clone().scrollbar_radius(radius);
        self
    }

    pub fn scrollbar_visibility(mut self, visibility: ScrollbarVisibilityStyle) -> Self {
        self.animated_style.base = self
            .animated_style
            .base
            .clone()
            .scrollbar_visibility(visibility);
        self
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn image_key(mut self, image_key: impl Into<ImageKey>) -> Self {
        self.image_key = image_key.into();
        self
    }

    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    pub fn into_element_desc(self, children: Vec<ElementDesc>) -> ElementDesc {
        widget_element_desc(self, children)
    }

    event_handler_methods!();
}

impl Default for ImageWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for ImageWidget {
    fn node_type(&self) -> WidgetType {
        WidgetType::Image
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn props_hash(&self) -> u64 {
        props_hash(&(
            &self.image_key,
            self.opacity.to_bits(),
            &self.animated_style,
        ))
    }

    fn update_from(&mut self, next: &Self) -> DirtyFlags {
        let mut flags = DirtyFlags::empty();
        if self.image_key != next.image_key {
            self.image_key = next.image_key.clone();
            flags |= DirtyFlags::PAINT;
        }
        if self.opacity.to_bits() != next.opacity.to_bits() {
            self.opacity = next.opacity;
            flags |= DirtyFlags::PAINT;
        }
        if self.animated_style != next.animated_style {
            self.animated_style = next.animated_style.clone();
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
        &self.animated_style.base
    }

    fn paint(&self, rect: Rect, style: &ComputedStyle, commands: &mut Vec<PaintCommand>) {
        paint_box(rect, style, commands);
        commands.push(PaintCommand::Image(ImagePaintCommand {
            key: self.image_key.clone(),
            rect,
            opacity: self.opacity,
        }));
    }

    fn handle_event(&mut self, _event: EventRef<'_>, _cx: &mut EventContext<'_>) -> EventResult {
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
