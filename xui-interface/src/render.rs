use crate::{
    Color, ComputedColorStyle, ComputedShadowStyle, ComputedStrokeStyle, ComputedTextStyle,
    LineHeight, NodeId, NodeLifecycleEvent, Point, Rect, Size, TextDecoration, TextRange,
    Translation,
};
use std::{
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

pub trait Painter {
    fn push(&mut self, command: PaintCommand);
}

impl Painter for Vec<PaintCommand> {
    fn push(&mut self, command: PaintCommand) {
        Vec::push(self, command);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaintCommand {
    Rect {
        rect: Rect,
        color: ComputedColorStyle,
        stroke: Option<ComputedStrokeStyle>,
        shadow: Option<ComputedShadowStyle>,
    },
    RoundedRect {
        rect: Rect,
        radius: f32,
        color: ComputedColorStyle,
        stroke: Option<ComputedStrokeStyle>,
        shadow: Option<ComputedShadowStyle>,
    },
    Line {
        from: Point,
        to: Point,
        color: Color,
        width: f32,
    },
    Text(TextPaintCommand),
    Image(ImagePaintCommand),
    // Clip
    PushClip(Rect),
    PopClip,

    // Transform
    PushTransform {
        translate: Translation,
    },
    PopTransform,

    Clear(Color),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextPaintCommand {
    pub node_id: NodeId,
    pub rect: Rect,
    pub paint: TextPaintProps,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextPaintProps {
    pub style: TextPaintStyle,
    pub caret: Option<TextCaret>,
    pub selection: Option<TextSelectionPaint>,
    pub ime: Option<TextImePaint>,
}

impl TextPaintProps {
    pub fn new(style: TextPaintStyle) -> Self {
        Self {
            style,
            caret: None,
            selection: None,
            ime: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextPaintStyle {
    pub color: Color,
    pub font_size: f32,
    pub line_height: LineHeight,
    pub decoration: TextDecoration,
}

impl TextPaintStyle {
    pub fn from_computed(style: &ComputedTextStyle) -> Self {
        Self {
            color: style.color,
            font_size: style.font_size,
            line_height: style.line_height,
            decoration: style.decoration,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextCaret {
    pub char_index: usize,
    pub color: Color,
    pub width: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextSelectionPaint {
    pub range: TextRange,
    pub color: Color,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextImePaint {
    pub range: TextRange,
    pub underline_color: Color,
    pub underline_width: f32,
}

/// A paint command for drawing an image.
///
/// Carries both:
/// - an [`ImageKey`] identifying the image (used by the backend as a stable
///   cache key for the uploaded GPU texture), and
/// - an [`Arc<ImageData>`] containing the actual decoded pixel data, so the
///   backend never has to look outside the command to know what to draw.
///
/// The [`ImageStyle`] describes widget-level presentation such as fit,
/// alignment, repeat, and sampling. The [`ImageVariant`] carries lower-level
/// renderer options such as transform, target size, and color space. Both are
/// intentionally part of the command rather than the key, so that the same
/// source image can be drawn with different presentation options without
/// duplicating the underlying pixel data.
#[derive(Debug, Clone, PartialEq)]
pub struct ImagePaintCommand {
    pub key: ImageKey,
    pub data: ImageData,
    /// Image widget bounds. Backends use this as the style container for fit,
    /// alignment, repeat, and clipping.
    pub rect: Rect,
    pub opacity: f32,
    pub variant: ImageVariant,
    pub style: ImageStyle,
}

/// Stable identifier for an image source.
///
/// `ImageKey` is intentionally a pure identity: it answers the question
/// "which image is this?" but says nothing about how to display it.
/// Display-time options (sampling, rotation, target size, ...) live on
/// [`ImageVariant`] inside [`ImagePaintCommand`].
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum ImageKey {
    AssetId([u8; 16]),
    AssetPath(PathBuf),
    Url(String),
    BytesHash(u64),
    UserProvided(u64),
}

impl Default for ImageKey {
    fn default() -> Self {
        ImageKey::UserProvided(0)
    }
}

impl From<&str> for ImageKey {
    fn from(value: &str) -> Self {
        if value.is_empty() {
            return Self::default();
        }
        ImageKey::AssetPath(PathBuf::from(value))
    }
}

impl From<String> for ImageKey {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

impl From<PathBuf> for ImageKey {
    fn from(value: PathBuf) -> Self {
        ImageKey::AssetPath(value)
    }
}

/// Low-level display-time options for an image draw.
///
/// These describe how a specific draw should transform / target-size the
/// underlying image without affecting the cached GPU texture identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ImageVariant {
    /// Desired rendered size in physical pixels, if the backend should
    /// resample to a specific size. `None` means "use natural size scaled to
    /// the destination rect".
    pub target_size_px: Option<(u32, u32)>,
    /// Scale factor (e.g. device pixel ratio) encoded as `f32::to_bits` so the
    /// struct can derive `Eq`/`Hash`.
    pub scale_factor_bits: u32,
    pub color_space: ColorSpace,
    pub sampling: Sampling,
    pub transform: ImageTransform,
}

impl ImageVariant {
    pub fn scale_factor(&self) -> f32 {
        f32::from_bits(self.scale_factor_bits)
    }

    pub fn with_scale_factor(mut self, scale: f32) -> Self {
        self.scale_factor_bits = scale.to_bits();
        self
    }
}

impl Default for ImageVariant {
    fn default() -> Self {
        Self {
            target_size_px: None,
            scale_factor_bits: 1.0f32.to_bits(),
            color_space: ColorSpace::Srgb,
            sampling: Sampling::Linear,
            transform: ImageTransform::default(),
        }
    }
}

/// High-level presentation options for an image widget.
///
/// Defaults preserve the historical image behavior: stretch the image to fill
/// the widget bounds using linear sampling and no tiling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageStyle {
    pub fit: ImageFit,
    pub alignment: Alignment,
    pub repeat: ImageRepeat,
    pub sampling: Sampling,
}

impl Default for ImageStyle {
    fn default() -> Self {
        Self {
            fit: ImageFit::Fill,
            alignment: Alignment::CENTER,
            repeat: ImageRepeat::NoRepeat,
            sampling: Sampling::Linear,
        }
    }
}

impl Hash for ImageStyle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.fit.hash(state);
        self.alignment.hash(state);
        self.repeat.hash(state);
        self.sampling.hash(state);
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum ImageFit {
    /// Stretch to fill the widget bounds without preserving aspect ratio.
    Fill,
    /// Preserve aspect ratio and show the whole image, possibly leaving empty space.
    Contain,
    /// Preserve aspect ratio and cover the widget bounds, possibly cropping.
    Cover,
    /// Draw at the image's natural logical size.
    None,
    /// Draw at natural size unless the image must shrink to fit.
    ScaleDown,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum ImageRepeat {
    NoRepeat,
    Repeat,
    RepeatX,
    RepeatY,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Alignment {
    /// Horizontal placement factor: `0.0` is start, `0.5` is center, `1.0` is end.
    pub x: f32,
    /// Vertical placement factor: `0.0` is start, `0.5` is center, `1.0` is end.
    pub y: f32,
}

impl Alignment {
    pub const START: Self = Self::new(0.0, 0.0);
    pub const CENTER: Self = Self::new(0.5, 0.5);
    pub const END: Self = Self::new(1.0, 1.0);

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl Default for Alignment {
    fn default() -> Self {
        Self::CENTER
    }
}

impl Hash for Alignment {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_f32_canonical(self.x, state);
        hash_f32_canonical(self.y, state);
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum ColorSpace {
    Srgb,
    LinearSrgb,
    DisplayP3,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum Sampling {
    Nearest,
    Linear,
    Cubic,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct ImageTransform {
    pub flip_x: bool,
    pub flip_y: bool,
    pub rotate: ImageRotation,
}

impl Default for ImageTransform {
    fn default() -> Self {
        Self {
            flip_x: false,
            flip_y: false,
            rotate: ImageRotation::Deg0,
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum ImageRotation {
    Deg0,
    Deg90,
    Deg180,
    Deg270,
}

/// Decoded pixel data for an image.
///
/// This is the runtime payload carried inside an `Arc` so that widgets, the
/// `UiArena` shared image pool, and `ImagePaintCommand`s can all reference
/// the same pixels without copying.
///
/// Every `ImageData` carries a process-unique [`ImageDataId`] assigned at
/// construction time. Cloning the `Arc<ImageData>` preserves the same id,
/// so the backend can quickly detect "is this the same data I uploaded last
/// frame?" without having to hash pixel contents.
#[derive(Debug, Clone)]
pub struct ImageData {
    id: ImageDataId,
    pub size: Size<u32>,
    pub pixels: Arc<[u8]>,
    pub format: ImageFormat,
}

impl ImageData {
    pub fn new(size: Size<u32>, pixels: impl Into<Arc<[u8]>>, format: ImageFormat) -> Self {
        Self {
            id: ImageDataId::next(),
            size,
            pixels: pixels.into(),
            format,
        }
    }

    pub fn rgba8(size: Size<u32>, pixels: impl Into<Arc<[u8]>>) -> Self {
        Self::new(size, pixels, ImageFormat::Rgba8UnormSrgb)
    }

    pub fn id(&self) -> ImageDataId {
        self.id
    }
}

impl PartialEq for ImageData {
    fn eq(&self, other: &Self) -> bool {
        // Identity comparison: two `ImageData`s are considered equal iff they
        // share the same id. This is consistent with the way the backend uses
        // the id as a cache version key.
        self.id == other.id
    }
}

/// Process-unique identifier for an [`ImageData`].
///
/// Cloning an `Arc<ImageData>` preserves the same id, so the renderer can use
/// `(ImageKey, ImageDataId)` as a stable composite cache key for uploaded
/// textures: same id ⇒ same pixels ⇒ no re-upload needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageDataId(u64);

impl ImageDataId {
    fn next() -> Self {
        static NEXT_IMAGE_DATA_ID: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_IMAGE_DATA_ID.fetch_add(1, Ordering::Relaxed);
        ImageDataId(id)
    }

    pub fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    Rgba8UnormSrgb,
}

fn hash_f32_canonical<H: Hasher>(value: f32, state: &mut H) {
    let bits = if value == 0.0 {
        0.0f32.to_bits()
    } else {
        value.to_bits()
    };
    bits.hash(state);
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Damage {
    visual_rect: Rect,
    rect: Rect,
}

impl Damage {
    pub fn new(rect: Rect, visual_rect: Rect) -> Self {
        Self { rect, visual_rect }
    }

    pub fn full(size: Size<f32>) -> Self {
        Self {
            visual_rect: Rect::new(0., 0., size.width, size.height),
            rect: Rect::new(0., 0., size.width, size.height),
        }
    }

    pub fn rect(self) -> Rect {
        self.rect
    }

    pub fn visual_rect(self) -> Rect {
        self.visual_rect
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DamageRegion {
    rects: Vec<Damage>,
}

impl DamageRegion {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, damage: Damage) {
        if damage.visual_rect.width > 0.0 && damage.visual_rect.height > 0.0 {
            self.rects.push(damage);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    pub fn clear(&mut self) {
        self.rects.clear();
    }

    pub fn take(&mut self) -> Self {
        Self {
            rects: std::mem::take(&mut self.rects),
        }
    }

    pub fn damages(&self) -> &[Damage] {
        &self.rects
    }

    pub fn rects(&self) -> impl Iterator<Item = Rect> + '_ {
        self.rects.iter().map(|damage| damage.rect)
    }

    pub fn visual_rects(&self) -> impl Iterator<Item = Rect> + '_ {
        self.rects.iter().map(|damage| damage.visual_rect)
    }

    pub fn bounds(&self) -> Option<Rect> {
        self.rects.iter().map(|r| r.visual_rect).reduce(Rect::union)
    }

    pub fn intersects(&self, rect: Rect) -> bool {
        self.rects.iter().any(|d| d.visual_rect.intersects(rect))
    }
}

pub trait RenderBackend<T> {
    type Error;

    fn begin_frame(&mut self, size: Size<f32>) -> Result<(), Self::Error>;
    fn paint(
        &mut self,
        commands: &[PaintCommand],
        damage: &DamageRegion,
        text: &mut T,
    ) -> Result<(), Self::Error>;
    fn end_frame(&mut self) -> Result<(), Self::Error>;

    fn did_present(&self) -> bool {
        true
    }

    fn resize(&mut self, _size: Size<f32>) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_factor(&mut self, _factor: f32) -> Result<(), Self::Error> {
        Ok(())
    }

    fn handle_node_lifecycle(&mut self, _event: &NodeLifecycleEvent) {}
}

pub trait DrawBackend<T>: RenderBackend<T> {}

impl<T, B: RenderBackend<T>> DrawBackend<T> for B {}

#[derive(Debug, Clone, Default)]
pub struct MockRenderBackend {
    pub frame_size: Option<Size<f32>>,
    pub frames: usize,
    pub last_damage: DamageRegion,
    pub last_commands: Vec<PaintCommand>,
}

impl<T> RenderBackend<T> for MockRenderBackend {
    type Error = core::convert::Infallible;

    fn begin_frame(&mut self, size: Size<f32>) -> Result<(), Self::Error> {
        self.frame_size = Some(size);
        Ok(())
    }

    fn paint(
        &mut self,
        commands: &[PaintCommand],
        damage: &DamageRegion,
        _text: &mut T,
    ) -> Result<(), Self::Error> {
        self.last_commands = commands.to_vec();
        self.last_damage = damage.clone();
        Ok(())
    }

    fn end_frame(&mut self) -> Result<(), Self::Error> {
        self.frames += 1;
        Ok(())
    }
}

pub trait FontRenderBackend {
    type Error;
}
