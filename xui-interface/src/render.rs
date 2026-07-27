use crate::{Color, ComputedTextStyle, LineHeight, Point, Rect, Size, TextDecoration, TextRange};
use std::{
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
    pub xx: f32,
    pub yx: f32,
    pub xy: f32,
    pub yy: f32,
    pub dx: f32,
    pub dy: f32,
}

impl Affine {
    pub const IDENTITY: Self = Self {
        xx: 1.0,
        yx: 0.0,
        xy: 0.0,
        yy: 1.0,
        dx: 0.0,
        dy: 0.0,
    };

    pub const fn new(xx: f32, yx: f32, xy: f32, yy: f32, dx: f32, dy: f32) -> Self {
        Self {
            xx,
            yx,
            xy,
            yy,
            dx,
            dy,
        }
    }

    pub const fn translate(x: f32, y: f32) -> Self {
        Self::new(1.0, 0.0, 0.0, 1.0, x, y)
    }

    pub const fn scale(x: f32, y: f32) -> Self {
        Self::new(x, 0.0, 0.0, y, 0.0, 0.0)
    }

    pub fn then(self, next: Self) -> Self {
        Self::new(
            next.xx * self.xx + next.xy * self.yx,
            next.yx * self.xx + next.yy * self.yx,
            next.xx * self.xy + next.xy * self.yy,
            next.yx * self.xy + next.yy * self.yy,
            next.xx * self.dx + next.xy * self.dy + next.dx,
            next.yx * self.dx + next.yy * self.dy + next.dy,
        )
    }

    pub fn transform_point(self, point: Point) -> Point {
        Point::new(
            self.xx * point.x + self.xy * point.y + self.dx,
            self.yx * point.x + self.yy * point.y + self.dy,
        )
    }

    pub fn transform_rect(self, rect: Rect) -> Rect {
        let points = [
            self.transform_point(Point::new(rect.x, rect.y)),
            self.transform_point(Point::new(rect.x + rect.width, rect.y)),
            self.transform_point(Point::new(rect.x, rect.y + rect.height)),
            self.transform_point(Point::new(rect.x + rect.width, rect.y + rect.height)),
        ];
        let min_x = points
            .iter()
            .map(|point| point.x)
            .fold(f32::INFINITY, f32::min);
        let min_y = points
            .iter()
            .map(|point| point.y)
            .fold(f32::INFINITY, f32::min);
        let max_x = points
            .iter()
            .map(|point| point.x)
            .fold(f32::NEG_INFINITY, f32::max);
        let max_y = points
            .iter()
            .map(|point| point.y)
            .fold(f32::NEG_INFINITY, f32::max);
        Rect::new(min_x, min_y, max_x - min_x, max_y - min_y)
    }

    pub fn is_translation(self) -> bool {
        self.xx == 1.0 && self.xy == 0.0 && self.yx == 0.0 && self.yy == 1.0
    }

    pub fn is_axis_aligned(self) -> bool {
        self.xy == 0.0 && self.yx == 0.0
    }
}

impl Default for Affine {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathSegment {
    MoveTo(Point),
    LineTo(Point),
    QuadraticTo {
        control: Point,
        to: Point,
    },
    CubicTo {
        control1: Point,
        control2: Point,
        to: Point,
    },
    Close,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct PathDataId(u64);

impl PathDataId {
    fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Debug, Clone)]
pub struct PathData {
    id: PathDataId,
    segments: Arc<[PathSegment]>,
    bounds: Rect,
}

impl PathData {
    pub fn id(&self) -> PathDataId {
        self.id
    }
    pub fn segments(&self) -> &[PathSegment] {
        &self.segments
    }
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Conservative control-hull bounds. Curves may occupy less space, never
    /// more, which makes this suitable for culling and conservative visual bounds.
    pub fn bounds(&self) -> Rect {
        self.bounds
    }
}

impl PartialEq for PathData {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

#[derive(Debug, Default)]
pub struct PathBuilder {
    segments: Vec<PathSegment>,
}

impl PathBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn move_to(&mut self, point: Point) -> &mut Self {
        self.segments.push(PathSegment::MoveTo(point));
        self
    }
    pub fn line_to(&mut self, point: Point) -> &mut Self {
        self.segments.push(PathSegment::LineTo(point));
        self
    }
    pub fn quadratic_to(&mut self, control: Point, to: Point) -> &mut Self {
        self.segments.push(PathSegment::QuadraticTo { control, to });
        self
    }
    pub fn cubic_to(&mut self, control1: Point, control2: Point, to: Point) -> &mut Self {
        self.segments.push(PathSegment::CubicTo {
            control1,
            control2,
            to,
        });
        self
    }
    pub fn close(&mut self) -> &mut Self {
        self.segments.push(PathSegment::Close);
        self
    }
    pub fn build(self) -> PathData {
        let bounds = path_segment_bounds(&self.segments);
        PathData {
            id: PathDataId::next(),
            segments: self.segments.into(),
            bounds,
        }
    }
}

fn path_segment_bounds(segments: &[PathSegment]) -> Rect {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let mut include = |point: Point| {
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    };
    for segment in segments {
        match *segment {
            PathSegment::MoveTo(point) | PathSegment::LineTo(point) => include(point),
            PathSegment::QuadraticTo { control, to } => {
                include(control);
                include(to);
            }
            PathSegment::CubicTo {
                control1,
                control2,
                to,
            } => {
                include(control1);
                include(control2);
                include(to);
            }
            PathSegment::Close => {}
        }
    }
    if min_x.is_finite() {
        Rect::new(min_x, min_y, max_x - min_x, max_y - min_y)
    } else {
        Rect::ZERO
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum LineCap {
    Butt,
    Square,
    Round,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum LineJoin {
    Miter,
    Bevel,
    Round,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathFill {
    pub color: Color,
    pub rule: FillRule,
}

impl PathFill {
    pub const fn new(color: Color) -> Self {
        Self {
            color,
            rule: FillRule::NonZero,
        }
    }
    pub const fn rule(mut self, rule: FillRule) -> Self {
        self.rule = rule;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathStroke {
    pub color: Color,
    pub width: f32,
    pub cap: LineCap,
    pub join: LineJoin,
}

impl PathStroke {
    pub const fn new(color: Color, width: f32) -> Self {
        Self {
            color,
            width,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
        }
    }
    pub const fn cap(mut self, cap: LineCap) -> Self {
        self.cap = cap;
        self
    }
    pub const fn join(mut self, join: LineJoin) -> Self {
        self.join = join;
        self
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn path_builder_records_all_segments_and_clone_preserves_identity() {
        let mut builder = PathBuilder::new();
        builder
            .move_to(Point::new(1.0, 2.0))
            .line_to(Point::new(3.0, 4.0))
            .quadratic_to(Point::new(5.0, 6.0), Point::new(7.0, 8.0))
            .cubic_to(
                Point::new(9.0, 10.0),
                Point::new(11.0, 12.0),
                Point::new(13.0, 14.0),
            )
            .close();
        let path = builder.build();
        assert_eq!(path.segments().len(), 5);
        assert_eq!(path, path.clone());
        assert_ne!(path, PathBuilder::new().build());
    }

    #[test]
    fn affine_composition_applies_in_call_order() {
        let transform = Affine::translate(-1.0, -2.0)
            .then(Affine::scale(2.0, 3.0))
            .then(Affine::translate(10.0, 20.0));
        assert_eq!(
            transform.transform_point(Point::new(1.0, 2.0)),
            Point::new(10.0, 20.0)
        );
    }

    #[test]
    fn path_bounds_are_conservative_and_affine_transforms_rectangles() {
        let mut builder = PathBuilder::new();
        builder.move_to(Point::new(1.0, 2.0)).cubic_to(
            Point::new(-4.0, 8.0),
            Point::new(12.0, -3.0),
            Point::new(6.0, 5.0),
        );
        assert_eq!(builder.build().bounds(), Rect::new(-4.0, -3.0, 16.0, 11.0));
        assert_eq!(
            Affine::scale(2.0, 3.0)
                .then(Affine::translate(10.0, 20.0))
                .transform_rect(Rect::new(1.0, 2.0, 4.0, 5.0)),
            Rect::new(12.0, 26.0, 8.0, 15.0)
        );
    }
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

/// Stable identifier for an image source.
///
/// `ImageKey` is intentionally a pure identity: it answers the question
/// "which image is this?" but says nothing about how to display it.
/// Display-time options (sampling, rotation, target size, ...) live on
/// [`ImageVariant`] on the retained image primitive.
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
    pub const TOP_LEADING: Self = Self::new(0.0, 0.0);
    pub const TOP: Self = Self::new(0.5, 0.0);
    pub const TOP_TRAILING: Self = Self::new(1.0, 0.0);
    pub const LEADING: Self = Self::new(0.0, 0.5);
    pub const TRAILING: Self = Self::new(1.0, 0.5);
    pub const BOTTOM_LEADING: Self = Self::new(0.0, 1.0);
    pub const BOTTOM: Self = Self::new(0.5, 1.0);
    pub const BOTTOM_TRAILING: Self = Self::new(1.0, 1.0);

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
/// `UiArena` shared image pool, and retained image primitives can all reference
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

pub trait FontRenderBackend {
    type Error;
}
