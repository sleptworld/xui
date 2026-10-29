//! A decoded image, drawn over the decoration of its box.

use xui_interface::{
    Alignment, Bounds, ComputedStyle, ImageData, ImageFit, ImageKey, ImageRepeat, ImageStyle,
    ImageVariant, NodeId, Sampling, Size, WidgetUpdateFlags,
};
use xui_macros::widget;

use super::roles::{IntrinsicSize, PaintContent};
use crate::assets::{AssetId, load_image_asset, load_image_asset_path};
use crate::node::{Draft, WidgetInstance, WidgetT};
use crate::render::{ImagePrimitive, Primitive, RenderTreeWriter};

/// An image. Its pixels and their renderer cache key are its props; its box
/// is any box's, decoration and children included.
#[widget(
    style(
        box_size,
        margin,
        padding,
        position,
        display,
        typography,
        background,
        corner_radius,
        border,
        box_shadow,
        clip,
        visibility,
        transform,
        effects,
        scroll,
        cursor
    ),
    behavior(
        focus,
        accessibility,
        shortcuts,
        motion,
        focus_binding,
        scroll_binding,
        size_observation,
        events
    ),
    shorthands(sources)
)]
#[props(derive(Debug, Clone, PartialEq))]
pub struct Image {
    /// The renderer's cache key for the pixels.
    #[default(ImageKey::from(""))]
    #[dirty(PAINT_OUTPUT)]
    pub image_key: ImageKey,
    /// The decoded pixels. Compared by identity, not content.
    #[default]
    #[dirty(fn = image_data_dirty)]
    pub image_data: Option<ImageData>,
    /// Fit, alignment, repetition and sampling.
    #[default]
    #[dirty(PAINT_OUTPUT)]
    pub image_style: ImageStyle,
    /// Clamped to `0.0..=1.0` when painted.
    #[default(1.0)]
    #[dirty(PAINT_OUTPUT)]
    pub opacity: f32,
}

impl WidgetT for Image {
    type State = ();

    fn create_state(_: &ImageProps) {}
}

/// New pixels repaint; they lay out again only at a new natural size.
fn image_data_dirty(old: &Option<ImageData>, new: &Option<ImageData>) -> WidgetUpdateFlags {
    let size = |data: &Option<ImageData>| data.as_ref().map(|data| data.size);
    if size(old) == size(new) {
        WidgetUpdateFlags::PAINT_OUTPUT
    } else {
        WidgetUpdateFlags::PAINT_OUTPUT | WidgetUpdateFlags::LAYOUT_INPUT
    }
}

pub fn image() -> Draft<Image> {
    Draft::new(ImageProps::builder().build())
}

impl ImageProps {
    /// The decoded image's natural size in logical pixels.
    pub fn intrinsic_size(&self) -> Option<Size<f32>> {
        self.image_data
            .as_ref()
            .map(|data| Size::new(data.size.width as f32, data.size.height as f32))
    }
}

/// Ways of setting an image's pixels, or one part of its image style, at
/// once.
#[allow(non_camel_case_types)]
mod sources {
    use super::*;

    shorthand! {
        /// Loads an image asset from the asset manager installed for the
        /// current UI thread, every time the node is built; a component that
        /// builds often can load once with `HookContext::use_asset` and set
        /// `asset_data` instead.
        asset: Image => ImageProperties, AssetId, |props, asset| {
            let data = load_image_asset(asset);
            props.image_key = ImageKey::AssetId(*asset.as_bytes());
            props.image_data = data;
        }
    }

    shorthand! {
        /// An image asset already loaded, under the key `asset` would give it.
        asset_data: Image => ImageProperties, (AssetId, Option<ImageData>), |props, value| {
            props.image_key = ImageKey::AssetId(*value.0.as_bytes());
            props.image_data = value.1;
        }
    }

    shorthand! {
        /// Loads an image by its normalized path in the configured bundle.
        asset_path: Image => ImageProperties, String, |props, path| {
            props.image_data = load_image_asset_path(&path);
            props.image_key = ImageKey::AssetPath(path.into());
        }
    }

    shorthand! {
        /// Decoded pixels and their renderer cache key.
        source: Image => ImageProperties, (ImageKey, ImageData), |props, source| {
            props.image_key = source.0;
            props.image_data = Some(source.1);
        }
    }

    shorthand! { fit: Image => ImageProperties, ImageFit, |props, fit| props.image_style.fit = fit }
    shorthand! { alignment: Image => ImageProperties, Alignment, |props, alignment| props.image_style.alignment = alignment }
    shorthand! { repeat: Image => ImageProperties, ImageRepeat, |props, repeat| props.image_style.repeat = repeat }
    shorthand! { sampling: Image => ImageProperties, Sampling, |props, sampling| props.image_style.sampling = sampling }
}

impl IntrinsicSize for WidgetInstance<Image> {
    fn intrinsic_size(&self) -> Option<Size<f32>> {
        self.props.intrinsic_size()
    }
}

impl PaintContent for WidgetInstance<Image> {
    fn paint(
        &self,
        _node_id: NodeId,
        rect: Bounds,
        _style: &ComputedStyle,
        writer: &mut RenderTreeWriter<'_>,
    ) {
        let props = &self.props;
        if let Some(image_data) = props.image_data.as_ref() {
            writer
                .primitive(Primitive::Image(ImagePrimitive {
                    variant: ImageVariant {
                        sampling: props.image_style.sampling,
                        ..ImageVariant::default()
                    },
                    style: props.image_style,
                    data: image_data.clone(),
                    image: props.image_key.clone(),
                    bounds: rect,
                    opacity: props.opacity.clamp(0.0, 1.0),
                }))
                .expect("widget render tree must remain valid");
        }
    }
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

    fn mounted(draft: Draft<Image>) -> WidgetInstance<Image> {
        WidgetInstance {
            props: draft.storage.props,
            state: (),
        }
    }

    fn rendered_images(
        widget: &WidgetInstance<Image>,
        rect: Bounds,
        style: &ComputedStyle,
    ) -> Vec<ImagePrimitive> {
        let mut scene = crate::render::RenderScene::new();
        let parent = scene.insert_group();
        let mut writer = RenderTreeWriter::new(&mut scene, parent);
        widget.paint(NodeId::default(), rect, style, &mut writer);
        writer.finish().unwrap();
        scene
            .children(parent)
            .unwrap()
            .iter()
            .filter_map(|id| match &scene.node(*id).unwrap().kind {
                crate::render::RenderNodeKind::Primitive(node) => match &node.primitive {
                    Primitive::Image(image) => Some(image.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    #[test]
    fn decoded_data_controls_intrinsic_size_and_paint() {
        let data = pixels(3, 2, 255);
        let widget =
            mounted(image().with::<image::source>((ImageKey::UserProvided(7), data.clone())));
        assert_eq!(widget.intrinsic_size(), Some(Size::new(3.0, 2.0)));

        let style = ComputedStyle::initial(&xui_interface::Theme::default());
        let rect = Bounds::from_origin_size((1.0, 2.0), (30.0, 20.0));
        let images = rendered_images(&widget, rect, &style);
        let command = images.last().unwrap();
        assert_eq!(command.bounds, rect);
        assert_eq!(command.image, ImageKey::UserProvided(7));
        assert_eq!(&command.data, &data);
        assert_eq!(command.style, ImageStyle::default());
        assert_eq!(command.variant.sampling, Sampling::Linear);
    }

    /// What replacing the props of `old` with those of `new` dirties.
    fn update(old: &Draft<Image>, new: &Draft<Image>) -> WidgetUpdateFlags {
        use crate::node::HostNode;
        Image::props_dirty(&old.storage.props, &new.storage.props)
    }

    #[test]
    fn data_updates_relayout_only_when_intrinsic_size_changes() {
        let widget = image().with::<image::source>((ImageKey::from("first"), pixels(2, 2, 1)));
        let same_size = image().with::<image::source>((ImageKey::from("second"), pixels(2, 2, 2)));
        let flags = update(&widget, &same_size);
        assert!(flags.contains(WidgetUpdateFlags::PAINT_OUTPUT));
        assert!(!flags.contains(WidgetUpdateFlags::LAYOUT_INPUT));

        let different_size =
            image().with::<image::source>((ImageKey::from("third"), pixels(4, 3, 3)));
        let flags = update(&same_size, &different_size);
        assert!(flags.contains(WidgetUpdateFlags::PAINT_OUTPUT | WidgetUpdateFlags::LAYOUT_INPUT));
        assert_eq!(
            mounted(different_size).intrinsic_size(),
            Some(Size::new(4.0, 3.0))
        );
    }

    #[test]
    fn image_style_updates_repaint_without_relayout() {
        let data = pixels(2, 2, 1);
        let widget = image().with::<image::source>((ImageKey::from("image"), data.clone()));
        let next = image()
            .with::<image::source>((ImageKey::from("image"), data))
            .with::<image::fit>(ImageFit::Contain);

        let flags = update(&widget, &next);

        assert!(flags.contains(WidgetUpdateFlags::PAINT_OUTPUT));
        assert!(!flags.contains(WidgetUpdateFlags::LAYOUT_INPUT));
        assert_eq!(next.storage.props.image_style.fit, ImageFit::Contain);
    }

    #[test]
    fn contain_fit_preserves_aspect_ratio_and_alignment() {
        let widget = mounted(
            image()
                .with::<image::source>((ImageKey::from("image"), pixels(2, 1, 1)))
                .with::<image::fit>(ImageFit::Contain)
                .with::<image::sampling>(Sampling::Nearest),
        );
        let style = ComputedStyle::initial(&xui_interface::Theme::default());
        let images = rendered_images(
            &widget,
            Bounds::from_origin_size((0.0, 0.0), (100.0, 100.0)),
            &style,
        );
        assert_eq!(images.len(), 1);
        assert_eq!(
            images[0].bounds,
            Bounds::from_origin_size((0.0, 0.0), (100.0, 100.0))
        );
        assert_eq!(images[0].style.fit, ImageFit::Contain);
        assert_eq!(images[0].variant.sampling, Sampling::Nearest);
        assert_eq!(images[0].style.sampling, Sampling::Nearest);
    }

    #[test]
    fn cover_fit_is_carried_in_retained_image_style() {
        let widget = mounted(
            image()
                .with::<image::source>((ImageKey::from("image"), pixels(2, 1, 1)))
                .with::<image::fit>(ImageFit::Cover),
        );
        let style = ComputedStyle::initial(&xui_interface::Theme::default());
        let rect = Bounds::from_origin_size((0.0, 0.0), (100.0, 100.0));
        let images = rendered_images(&widget, rect, &style);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].bounds, rect);
        assert_eq!(images[0].style.fit, ImageFit::Cover);
    }

    #[test]
    fn repeat_x_is_carried_in_retained_image_style() {
        let widget = mounted(
            image()
                .with::<image::source>((ImageKey::from("image"), pixels(10, 10, 1)))
                .with::<image::fit>(ImageFit::None)
                .with::<image::alignment>(Alignment::START)
                .with::<image::repeat>(ImageRepeat::RepeatX),
        );
        let style = ComputedStyle::initial(&xui_interface::Theme::default());
        let images = rendered_images(
            &widget,
            Bounds::from_origin_size((0.0, 0.0), (25.0, 10.0)),
            &style,
        );
        assert_eq!(images.len(), 1);
        assert_eq!(
            images[0].bounds,
            Bounds::from_origin_size((0.0, 0.0), (25.0, 10.0))
        );
        assert_eq!(images[0].style.fit, ImageFit::None);
        assert_eq!(images[0].style.alignment, Alignment::START);
        assert_eq!(images[0].style.repeat, ImageRepeat::RepeatX);
    }

    #[test]
    fn scale_down_is_carried_in_retained_image_style() {
        let style = ComputedStyle::initial(&xui_interface::Theme::default());
        let widget = mounted(
            image()
                .with::<image::source>((ImageKey::from("image"), pixels(20, 10, 1)))
                .with::<image::fit>(ImageFit::ScaleDown),
        );

        let images = rendered_images(
            &widget,
            Bounds::from_origin_size((0.0, 0.0), (100.0, 100.0)),
            &style,
        );
        assert_eq!(images.len(), 1);
        assert_eq!(
            images[0].bounds,
            Bounds::from_origin_size((0.0, 0.0), (100.0, 100.0))
        );
        assert_eq!(images[0].style.fit, ImageFit::ScaleDown);
    }
}
