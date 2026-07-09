use crate::assets::{AssetId, load_image_asset, load_image_asset_path};
use crate::element::ElementDesc;
use crate::event_system::callbacks::EventHandlers;
use xui_interface::{style::ScrollbarStylePatch, *};

use super::{props_hash, widget_element_desc};

pub struct ImageWidget {
    pub key: Option<Key>,
    pub image_data: Option<ImageData>,
    pub image_key: ImageKey,
    pub image_style: ImageStyle,
    pub opacity: f32,
    pub style: Style,
    pub event_handlers: EventHandlers,
}

impl std::fmt::Debug for ImageWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageWidget")
            .field("key", &self.key)
            .field("image_key", &self.image_key)
            .field("image_style", &self.image_style)
            .field("opacity", &self.opacity)
            .field("style", &self.style)
            .finish()
    }
}

impl ImageWidget {
    pub fn new() -> Self {
        Self {
            key: None,
            image_key: "".into(),
            image_style: ImageStyle::default(),
            opacity: 1.0,
            style: Style::new(),
            event_handlers: EventHandlers::default(),
            image_data: None,
        }
    }

    pub fn with_image_key(image_key: impl Into<ImageKey>) -> Self {
        Self::new().image_key(image_key)
    }

    /// Loads an image from the asset manager installed for the current UI thread.
    pub fn asset(mut self, asset: AssetId) -> Self {
        self.image_key = ImageKey::AssetId(*asset.as_bytes());
        self.image_data = load_image_asset(asset);
        self
    }

    /// Loads an image by its normalized path in the configured asset bundle.
    pub fn asset_path(mut self, path: impl AsRef<str>) -> Self {
        let path = path.as_ref();
        self.image_key = ImageKey::AssetPath(path.into());
        self.image_data = load_image_asset_path(path);
        self
    }

    /// Supplies already-decoded pixels and their stable renderer cache key.
    pub fn image_data(
        mut self,
        image_key: impl Into<ImageKey>,
        image_data: impl Into<ImageData>,
    ) -> Self {
        self.image_key = image_key.into();
        self.image_data = Some(image_data.into());
        self
    }

    /// Returns the decoded image's natural size in logical pixels.
    pub fn intrinsic_size(&self) -> Option<Size<f32>> {
        self.image_data
            .as_ref()
            .map(|data| Size::new(data.size.width as f32, data.size.height as f32))
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

    pub fn image_key(mut self, image_key: impl Into<ImageKey>) -> Self {
        self.image_key = image_key.into();
        self
    }

    pub fn image_style(mut self, image_style: ImageStyle) -> Self {
        self.image_style = image_style;
        self
    }

    pub fn fit(mut self, fit: ImageFit) -> Self {
        self.image_style.fit = fit;
        self
    }

    pub fn alignment(mut self, alignment: Alignment) -> Self {
        self.image_style.alignment = alignment;
        self
    }

    pub fn repeat(mut self, repeat: ImageRepeat) -> Self {
        self.image_style.repeat = repeat;
        self
    }

    pub fn sampling(mut self, sampling: Sampling) -> Self {
        self.image_style.sampling = sampling;
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
            self.image_data.as_ref().map(|data| data.id().raw()),
            &self.image_style,
            self.opacity.to_bits(),
            &self.style,
        ))
    }

    fn update_from(&mut self, next: &Self) -> WidgetUpdateFlags {
        let mut flags = WidgetUpdateFlags::empty();
        if self.image_key != next.image_key {
            self.image_key = next.image_key.clone();
            flags |= WidgetUpdateFlags::PAINT_OUTPUT;
        }
        let current_data_id = self.image_data.as_ref().map(|data| data.id());
        let next_data_id = next.image_data.as_ref().map(|data| data.id());
        if current_data_id != next_data_id {
            let current_size = self.intrinsic_size();
            let next_size = next.intrinsic_size();
            self.image_data = next.image_data.clone();
            flags |= WidgetUpdateFlags::PAINT_OUTPUT;
            if current_size != next_size {
                flags |= WidgetUpdateFlags::LAYOUT_INPUT;
            }
        }
        if self.opacity.to_bits() != next.opacity.to_bits() {
            self.opacity = next.opacity;
            flags |= WidgetUpdateFlags::PAINT_OUTPUT;
        }
        if self.image_style != next.image_style {
            self.image_style = next.image_style;
            flags |= WidgetUpdateFlags::PAINT_OUTPUT;
        }
        if self.style != next.style {
            self.style = next.style.clone();
            flags |= WidgetUpdateFlags::STYLE_TARGET;
        }

        if flags.is_empty() {
            WidgetUpdateFlags::empty()
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

        if let Some(image_data) = self.image_data.as_ref() {
            commands.push(PaintCommand::Image(ImagePaintCommand {
                variant: ImageVariant {
                    sampling: self.image_style.sampling,
                    ..ImageVariant::default()
                },
                style: self.image_style,
                data: image_data.clone(),
                key: self.image_key.clone(),
                rect,
                opacity: self.opacity,
            }));
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pixels(width: u32, height: u32, value: u8) -> ImageData {
        ImageData::rgba8(
            Size::new(width, height),
            vec![value; width as usize * height as usize * 4],
        )
    }

    fn image_commands(commands: &[PaintCommand]) -> Vec<&ImagePaintCommand> {
        commands
            .iter()
            .filter_map(|command| match command {
                PaintCommand::Image(command) => Some(command),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn decoded_data_controls_intrinsic_size_and_paint() {
        let data = pixels(3, 2, 255);
        let widget = ImageWidget::new().image_data(ImageKey::UserProvided(7), data.clone());
        assert_eq!(widget.intrinsic_size(), Some(Size::new(3.0, 2.0)));

        let style = ComputedStyle::initial(&Theme::default());
        let rect = Rect::new(1.0, 2.0, 30.0, 20.0);
        let mut commands = Vec::new();
        widget.paint(rect, &style, &mut commands);

        let PaintCommand::Image(command) = commands.last().unwrap() else {
            panic!("expected image paint command");
        };
        assert_eq!(command.rect, rect);
        assert_eq!(command.key, ImageKey::UserProvided(7));
        assert_eq!(&command.data, &data);
        assert_eq!(command.style, ImageStyle::default());
        assert_eq!(command.variant.sampling, Sampling::Linear);
    }

    #[test]
    fn data_updates_relayout_only_when_intrinsic_size_changes() {
        let mut widget = ImageWidget::new().image_data("first", pixels(2, 2, 1));
        let same_size = ImageWidget::new().image_data("second", pixels(2, 2, 2));
        let flags = widget.update_from(&same_size);
        assert!(flags.contains(WidgetUpdateFlags::PAINT_OUTPUT));
        assert!(!flags.contains(WidgetUpdateFlags::LAYOUT_INPUT));

        let different_size = ImageWidget::new().image_data("third", pixels(4, 3, 3));
        let flags = widget.update_from(&different_size);
        assert!(flags.contains(WidgetUpdateFlags::PAINT_OUTPUT | WidgetUpdateFlags::LAYOUT_INPUT));
        assert_eq!(widget.intrinsic_size(), Some(Size::new(4.0, 3.0)));
    }

    #[test]
    fn image_style_updates_repaint_without_relayout() {
        let mut widget = ImageWidget::new().image_data("image", pixels(2, 2, 1));
        let next = ImageWidget::new()
            .image_data("image", widget.image_data.clone().unwrap())
            .fit(ImageFit::Contain);

        let flags = widget.update_from(&next);

        assert!(flags.contains(WidgetUpdateFlags::PAINT_OUTPUT));
        assert!(!flags.contains(WidgetUpdateFlags::LAYOUT_INPUT));
        assert_eq!(widget.image_style.fit, ImageFit::Contain);
    }

    #[test]
    fn contain_fit_preserves_aspect_ratio_and_alignment() {
        let widget = ImageWidget::new()
            .image_data("image", pixels(2, 1, 1))
            .fit(ImageFit::Contain)
            .sampling(Sampling::Nearest);
        let style = ComputedStyle::initial(&Theme::default());
        let mut commands = Vec::new();

        widget.paint(Rect::new(0.0, 0.0, 100.0, 100.0), &style, &mut commands);

        let images = image_commands(&commands);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].rect, Rect::new(0.0, 0.0, 100.0, 100.0));
        assert_eq!(images[0].style.fit, ImageFit::Contain);
        assert_eq!(images[0].variant.sampling, Sampling::Nearest);
        assert_eq!(images[0].style.sampling, Sampling::Nearest);
    }

    #[test]
    fn cover_fit_is_carried_in_paint_command_style() {
        let widget = ImageWidget::new()
            .image_data("image", pixels(2, 1, 1))
            .fit(ImageFit::Cover);
        let style = ComputedStyle::initial(&Theme::default());
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let mut commands = Vec::new();

        widget.paint(rect, &style, &mut commands);

        let images = image_commands(&commands);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].rect, rect);
        assert_eq!(images[0].style.fit, ImageFit::Cover);
    }

    #[test]
    fn repeat_x_is_carried_in_paint_command_style() {
        let widget = ImageWidget::new()
            .image_data("image", pixels(10, 10, 1))
            .fit(ImageFit::None)
            .alignment(Alignment::START)
            .repeat(ImageRepeat::RepeatX);
        let style = ComputedStyle::initial(&Theme::default());
        let mut commands = Vec::new();

        widget.paint(Rect::new(0.0, 0.0, 25.0, 10.0), &style, &mut commands);

        let images = image_commands(&commands);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].rect, Rect::new(0.0, 0.0, 25.0, 10.0));
        assert_eq!(images[0].style.fit, ImageFit::None);
        assert_eq!(images[0].style.alignment, Alignment::START);
        assert_eq!(images[0].style.repeat, ImageRepeat::RepeatX);
    }

    #[test]
    fn scale_down_is_carried_in_paint_command_style() {
        let style = ComputedStyle::initial(&Theme::default());
        let mut commands = Vec::new();
        let widget = ImageWidget::new()
            .image_data("image", pixels(20, 10, 1))
            .fit(ImageFit::ScaleDown);

        widget.paint(Rect::new(0.0, 0.0, 100.0, 100.0), &style, &mut commands);

        let images = image_commands(&commands);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].rect, Rect::new(0.0, 0.0, 100.0, 100.0));
        assert_eq!(images[0].style.fit, ImageFit::ScaleDown);
    }
}
