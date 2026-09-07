//! Retained vector drawing.
//!
//! Three things a canvas has to get right, kept separate here because folding
//! them into one retained scene is what made the old design awkward:
//!
//! **Where the drawing comes from.** [`CanvasContent::Scene`] is authored by
//! the caller before layout, so it can only use sizes the caller already knows.
//! [`CanvasContent::Painter`] is a closure evaluated *after* layout, handed the
//! node's measured size, its resolved style, and the scale factor -- everything
//! a drawing needs to fill the box it was actually given instead of a box its
//! author guessed at.
//!
//! **How a change reaches the screen.** A canvas is a leaf: its content changes
//! nothing about layout or the component tree. So [`CanvasController`] marks its
//! host nodes directly through [`CanvasInvalidator`], and the runtime reports
//! that pending work from `is_dirty`. No component rebuilds, and the content
//! revision stays out of `props_hash` for the same reason.
//!
//! **What the renderer is asked to do.** Commands lower to render primitives in
//! runs of adjacent same-kind commands. Analytic shapes become instanced SDF
//! quads; only paths open a vector run, which is the one expensive kind. The
//! order a chart naturally emits in -- grid shapes, series paths, marker shapes,
//! labels -- costs exactly one vector run. Lowering happens once per compile,
//! so a repaint reuses the scenes it built, and with them every compiled path
//! the backend has cached.

use std::{cell::RefCell, collections::HashMap, fmt, rc::Rc, sync::Arc};

use crate::element::ElementDesc;
use crate::event_system::interaction::InteractionProperties;
use crate::event_system::{EventContext, callbacks::EventHandlers};
use crate::render::{
    ClipShape, Primitive, RenderTreeWriter, ShapePrimitive, TextPrimitive, VectorPrimitive,
};
use crate::text::TextLayoutSlot;
#[cfg(feature = "wgpu")]
use xui_interface::ExternalTextureHandle;
use xui_interface::{
    Affine, Bounds, CanvasTextId, Color, ComputedColorStyle, ComputedStrokeStyle, ComputedStyle,
    EventRef, EventResult, ExternalTextureId, Key, NodeId, PathData, PathFill, PathStroke, Point,
    Sampling, Shape, Size, StrokeLineStyle, Style, TextContent, TextLayoutConstraints,
    TextPaintProps, TextPaintStyle, TextProps, Theme, VectorCommand, VectorScene, WidgetType,
    WidgetUpdateFlags,
};

use super::{props_hash, widget_element_desc};

/// Caller-chosen identity for a pickable region. See [`CanvasController::pick`].
pub type CanvasPickTag = u32;

/// Explicitly keyed text ids live in the top half of the id space so they can
/// never collide with the ones [`CanvasPainter`] hands out by position.
const KEYED_TEXT_ID_BIT: u32 = 1 << 31;

/// Where a canvas's drawing comes from.
///
/// A retained scene is authored before layout runs, so it can only use sizes
/// the caller already knows -- which is why size-dependent drawings used to end
/// up with their dimensions hard-coded. A painter is evaluated *after* layout
/// with the node's measured size, so it can fill whatever box it was given.
#[derive(Clone)]
pub enum CanvasContent {
    Scene(VectorScene),
    Painter(Rc<dyn Fn(&mut CanvasPainter<'_>)>),
    /// Drawn by the application's own GPU code. See
    /// [`CanvasController::with_gpu_painter`].
    #[cfg(feature = "wgpu")]
    GpuPainter(Rc<dyn GpuPainter>),
}

impl CanvasContent {
    /// Whether a change in the node's measured size invalidates the drawing.
    fn is_size_dependent(&self) -> bool {
        match self {
            Self::Scene(_) => false,
            Self::Painter(_) => true,
            // The target is allocated to the node's measured size, so a resize
            // is a reallocation and a redraw.
            #[cfg(feature = "wgpu")]
            Self::GpuPainter(_) => true,
        }
    }
}

impl fmt::Debug for CanvasContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scene(scene) => f.debug_tuple("Scene").field(scene).finish(),
            Self::Painter(_) => f.write_str("Painter(..)"),
            #[cfg(feature = "wgpu")]
            Self::GpuPainter(_) => f.write_str("GpuPainter(..)"),
        }
    }
}

impl Default for CanvasContent {
    fn default() -> Self {
        Self::Scene(VectorScene::default())
    }
}

/// The GPU device a canvas GPU painter draws with.
///
/// Handed to the runtime by the renderer at startup, because it must be the
/// *renderer's* device: a texture from any other one cannot be composited
/// without a copy, which is the whole point of this path. Without the `wgpu`
/// feature the type still exists but cannot be built, so a GPU painter simply
/// never runs.
#[derive(Clone)]
pub struct CanvasGpuContext {
    #[cfg(feature = "wgpu")]
    device: wgpu::Device,
    #[cfg(feature = "wgpu")]
    queue: wgpu::Queue,
}

#[cfg(feature = "wgpu")]
impl CanvasGpuContext {
    /// Wraps the renderer's device. Under `xui-skia`, the two handles come from
    /// `SkiaBackend::wgpu_context`.
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self { device, queue }
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }
}

impl fmt::Debug for CanvasGpuContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CanvasGpuContext")
    }
}

/// The format a canvas GPU target is allocated with, and the one a pipeline
/// drawing into it must declare.
///
/// sRGB rather than linear so that a shader writing `1.0` produces the same
/// colour as `Color::WHITE` elsewhere in the UI: the hardware encodes on write
/// and the renderer decodes on sample, so values round-trip instead of being
/// gamma-shifted by the compositing step.
#[cfg(feature = "wgpu")]
pub const CANVAS_GPU_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Usages a canvas GPU target is allocated with: the painter renders into it
/// and the renderer samples it.
#[cfg(feature = "wgpu")]
pub const CANVAS_GPU_USAGES: wgpu::TextureUsages =
    wgpu::TextureUsages::RENDER_ATTACHMENT.union(wgpu::TextureUsages::TEXTURE_BINDING);

/// The GPU drawing surface handed to a [`CanvasContent::GpuPainter`] each draw.
///
/// The target is the framework's: allocated at the node's measured size in
/// physical pixels, reallocated when that changes, and composited into the
/// surrounding UI afterwards. A draw records a render pass into
/// [`Self::target`] and submits on [`Self::queue`]; nothing has to be
/// registered by hand.
///
/// The painter's own state -- pipelines, layouts, buffers -- is kept by the
/// canvas across draws; see [`CanvasController::with_gpu_painter`].
#[cfg(feature = "wgpu")]
pub struct CanvasGpuPainter<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    target: &'a wgpu::TextureView,
    size: Size<u32>,
    logical_size: Size<f32>,
    scale_factor: f32,
    style: &'a ComputedStyle,
    theme: &'a Theme,
    picks: Vec<CanvasPick>,
}

#[cfg(feature = "wgpu")]
impl<'a> CanvasGpuPainter<'a> {
    /// The renderer's device. Pipelines and buffers must be created on it.
    pub fn device(&self) -> &'a wgpu::Device {
        self.device
    }

    /// The renderer's queue. Submitting here is enough -- the renderer samples
    /// the target on the same queue, so submission order does the ordering.
    pub fn queue(&self) -> &'a wgpu::Queue {
        self.queue
    }

    /// The colour attachment to render into, in [`CANVAS_GPU_FORMAT`].
    pub fn target(&self) -> &'a wgpu::TextureView {
        self.target
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        CANVAS_GPU_FORMAT
    }

    /// The target's size in physical pixels -- the viewport a pipeline should
    /// assume.
    pub fn size(&self) -> Size<u32> {
        self.size
    }

    /// The node's measured size in logical pixels, which is the rectangle the
    /// result is composited into.
    pub fn logical_size(&self) -> Size<f32> {
        self.logical_size
    }

    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    pub fn style(&self) -> &'a ComputedStyle {
        self.style
    }

    pub fn theme(&self) -> &'a Theme {
        self.theme
    }

    /// Records a region that [`CanvasController::pick`] can hit-test, in
    /// logical pixels. Same semantics as [`CanvasPainter::pick`].
    pub fn pick(&mut self, tag: CanvasPickTag, bounds: Bounds) -> &mut Self {
        self.picks.push(CanvasPick { tag, bounds });
        self
    }
}

/// A canvas GPU painter, with its state type erased.
///
/// Implemented once, for the closure [`CanvasController::with_gpu_painter`]
/// takes. It exists so that [`CanvasContent`] can hold a painter without naming
/// the caller's state type.
#[cfg(feature = "wgpu")]
pub trait GpuPainter {
    fn draw(&self, painter: &mut CanvasGpuPainter<'_>);
}

/// A GPU painter's closure and the state it keeps between draws.
///
/// Living inside the `Rc` that holds the closure is what keeps it correct:
/// replacing a controller's content drops the state with it, so nothing has to
/// track which pipelines belonged to which painter.
#[cfg(feature = "wgpu")]
struct GpuPainterFn<S, Draw> {
    draw: Draw,
    state: RefCell<Option<S>>,
}

#[cfg(feature = "wgpu")]
impl<S, Draw> GpuPainter for GpuPainterFn<S, Draw>
where
    Draw: Fn(&mut Option<S>, &mut CanvasGpuPainter<'_>),
{
    fn draw(&self, painter: &mut CanvasGpuPainter<'_>) {
        (self.draw)(&mut self.state.borrow_mut(), painter);
    }
}

/// A pickable region recorded while painting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasPick {
    pub tag: CanvasPickTag,
    pub bounds: Bounds,
}

/// Geometry produced by shaping text for a canvas.
///
/// Values are in logical pixels and describe layout geometry, not rasterized
/// glyph (ink) bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasTextMetrics {
    pub size: Size<f32>,
    pub first_baseline: Option<f32>,
    pub line_count: usize,
}

/// A shaped canvas text layout ready to be positioned and drawn.
///
/// Construct one with [`CanvasPainter::layout_text`] or
/// [`CanvasPainter::layout_text_keyed`]. Keeping measurement and drawing in one
/// value guarantees that both operations use the same text slot and wrapping
/// constraint.
#[derive(Debug)]
pub struct CanvasTextLayout {
    id: CanvasTextId,
    props: Arc<TextProps>,
    constraints: TextLayoutConstraints,
    metrics: CanvasTextMetrics,
}

impl CanvasTextLayout {
    pub fn id(&self) -> CanvasTextId {
        self.id
    }

    pub fn metrics(&self) -> CanvasTextMetrics {
        self.metrics
    }

    pub fn constraints(&self) -> TextLayoutConstraints {
        self.constraints
    }

    /// Size of the layout box used when drawing at an origin.
    ///
    /// A definite width is preserved even when the visible text is narrower,
    /// so paragraph alignment behaves exactly as it did during shaping.
    pub fn box_size(&self) -> Size<f32> {
        let width = match self.constraints {
            TextLayoutConstraints::Definate(width) => width.max(0.0),
            TextLayoutConstraints::Unbound | TextLayoutConstraints::MinSize => {
                self.metrics.size.width
            }
        };
        Size::new(width, self.metrics.size.height)
    }
}

/// The drawing surface handed to a [`CanvasContent::Painter`].
///
/// Everything here is in canvas-local coordinates: the origin is the node's
/// top-left corner, and [`CanvasPainter::size`] is its measured size.
pub struct CanvasPainter<'a> {
    commands: Vec<VectorCommand>,
    picks: Vec<CanvasPick>,
    size: Size<f32>,
    style: &'a ComputedStyle,
    theme: &'a Theme,
    scale_factor: f32,
    next_text_id: u32,
    text_constraints: HashMap<CanvasTextId, TextLayoutConstraints>,
    measure_text:
        &'a mut dyn FnMut(CanvasTextId, &TextProps, TextLayoutConstraints) -> CanvasTextMetrics,
}

impl<'a> CanvasPainter<'a> {
    fn new(
        size: Size<f32>,
        style: &'a ComputedStyle,
        theme: &'a Theme,
        scale_factor: f32,
        measure_text: &'a mut dyn FnMut(
            CanvasTextId,
            &TextProps,
            TextLayoutConstraints,
        ) -> CanvasTextMetrics,
    ) -> Self {
        Self {
            commands: Vec::new(),
            picks: Vec::new(),
            size,
            style,
            theme,
            scale_factor,
            next_text_id: 1,
            text_constraints: HashMap::new(),
            measure_text,
        }
    }

    fn next_text_id(&mut self) -> CanvasTextId {
        let id = CanvasTextId::new(self.next_text_id);
        self.next_text_id = self.next_text_id.wrapping_add(1) & !KEYED_TEXT_ID_BIT;
        id
    }

    fn keyed_text_id(key: u32) -> CanvasTextId {
        CanvasTextId::new((key & !KEYED_TEXT_ID_BIT) | KEYED_TEXT_ID_BIT)
    }

    /// The node's measured size. This is the whole point of a painter.
    pub fn size(&self) -> Size<f32> {
        self.size
    }

    pub fn width(&self) -> f32 {
        self.size.width
    }

    pub fn height(&self) -> f32 {
        self.size.height
    }

    /// The full drawing area, with its origin at zero.
    pub fn bounds(&self) -> Bounds {
        Bounds::from_origin_size(Point::new(0.0, 0.0), self.size)
    }

    /// The node's resolved style, so a drawing can follow the surrounding UI
    /// instead of hard-coding colors that a theme switch will not reach.
    pub fn style(&self) -> &ComputedStyle {
        self.style
    }

    pub fn theme(&self) -> &Theme {
        self.theme
    }

    /// The inherited text color, the canvas equivalent of `currentColor`.
    pub fn color(&self) -> Color {
        self.style.text.color
    }

    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    /// The thinnest line that still lands on a whole device pixel.
    pub fn hairline(&self) -> f32 {
        1.0 / self.scale_factor.max(f32::EPSILON)
    }

    /// Snaps a coordinate to a device pixel edge, so a one-pixel rule does not
    /// smear across two rows.
    pub fn snap(&self, value: f32) -> f32 {
        let scale = self.scale_factor.max(f32::EPSILON);
        (value * scale).round() / scale
    }

    /// Draws an analytic shape. Cheaper than the equivalent path by an order of
    /// magnitude: shapes go to the instanced SDF pipeline instead of starting a
    /// vector run, and they carry gradients that [`PathFill`] cannot.
    pub fn shape(
        &mut self,
        bounds: Bounds,
        shape: Shape,
        fill: Option<ComputedColorStyle>,
        stroke: Option<ComputedStrokeStyle>,
    ) -> &mut Self {
        self.commands.push(VectorCommand::Shape {
            bounds,
            shape,
            fill,
            stroke,
        });
        self
    }

    pub fn rect(&mut self, bounds: Bounds, fill: impl Into<ComputedColorStyle>) -> &mut Self {
        self.shape(bounds, Shape::Rect, Some(fill.into()), None)
    }

    pub fn rounded_rect(
        &mut self,
        bounds: Bounds,
        radius: f32,
        fill: impl Into<ComputedColorStyle>,
    ) -> &mut Self {
        self.shape(bounds, Shape::RoundedRect(radius), Some(fill.into()), None)
    }

    pub fn circle(
        &mut self,
        center: Point,
        radius: f32,
        fill: impl Into<ComputedColorStyle>,
    ) -> &mut Self {
        let bounds = Bounds::from_origin_size(
            Point::new(center.x - radius, center.y - radius),
            Size::new(radius * 2.0, radius * 2.0),
        );
        self.shape(bounds, Shape::Circle, Some(fill.into()), None)
    }

    /// A straight rule. Prefer this over a two-point path: it never starts a
    /// vector run.
    pub fn line(&mut self, from: Point, to: Point, color: Color, width: f32) -> &mut Self {
        let bounds = Bounds::new(
            Point::new(from.x.min(to.x), from.y.min(to.y)),
            Point::new(from.x.max(to.x), from.y.max(to.y)),
        );
        self.shape(
            bounds,
            Shape::Line { from, to },
            None,
            Some(ComputedStrokeStyle {
                color: ComputedColorStyle::Solid(color),
                width,
                line_style: StrokeLineStyle::Solid,
            }),
        )
    }

    pub fn fill_path(&mut self, path: PathData, fill: PathFill) -> &mut Self {
        self.fill_path_with(path, Affine::IDENTITY, fill)
    }

    pub fn fill_path_with(
        &mut self,
        path: PathData,
        transform: Affine,
        fill: PathFill,
    ) -> &mut Self {
        self.commands.push(VectorCommand::FillPath {
            path,
            transform,
            fill,
        });
        self
    }

    pub fn stroke_path(&mut self, path: PathData, stroke: PathStroke) -> &mut Self {
        self.stroke_path_with(path, Affine::IDENTITY, stroke)
    }

    pub fn stroke_path_with(
        &mut self,
        path: PathData,
        transform: Affine,
        stroke: PathStroke,
    ) -> &mut Self {
        self.commands.push(VectorCommand::StrokePath {
            path,
            transform,
            stroke,
        });
        self
    }

    /// Places a text box, returning the id it was given.
    ///
    /// The id is allocated by emission order, so text that keeps its position
    /// in the drawing keeps its shaping slot across repaints. Use
    /// [`CanvasPainter::text_keyed`] when the order itself moves.
    pub fn text(&mut self, bounds: Bounds, props: TextProps) -> CanvasTextId {
        let id = self.next_text_id();
        self.push_text(id, bounds, props);
        id
    }

    /// Places a text box under a caller-chosen key, for labels whose emission
    /// order changes between frames. Keys are namespaced away from the
    /// automatic ids, so the two schemes can be mixed in one drawing.
    pub fn text_keyed(&mut self, key: u32, bounds: Bounds, props: TextProps) -> CanvasTextId {
        let id = Self::keyed_text_id(key);
        self.push_text(id, bounds, props);
        id
    }

    /// Shapes text without drawing it and returns its layout geometry.
    ///
    /// The returned layout owns the same stable slot that
    /// [`CanvasPainter::draw_text`] later activates for paint. Measurement uses
    /// the application's real text backend and font fallback rules.
    pub fn layout_text(
        &mut self,
        props: TextProps,
        constraints: TextLayoutConstraints,
    ) -> CanvasTextLayout {
        let id = self.next_text_id();
        self.layout_text_with_id(id, props, constraints)
    }

    /// Keyed form of [`CanvasPainter::layout_text`] for labels whose call order
    /// changes between canvas compilations.
    pub fn layout_text_keyed(
        &mut self,
        key: u32,
        props: TextProps,
        constraints: TextLayoutConstraints,
    ) -> CanvasTextLayout {
        self.layout_text_with_id(Self::keyed_text_id(key), props, constraints)
    }

    fn layout_text_with_id(
        &mut self,
        id: CanvasTextId,
        props: TextProps,
        constraints: TextLayoutConstraints,
    ) -> CanvasTextLayout {
        let metrics = (self.measure_text)(id, &props, constraints);
        CanvasTextLayout {
            id,
            props: Arc::new(props),
            constraints,
            metrics,
        }
    }

    /// Draws a previously shaped layout with its layout-box origin at `origin`.
    pub fn draw_text(&mut self, origin: Point, layout: CanvasTextLayout) -> CanvasTextId {
        let id = layout.id;
        let bounds = Bounds::from_origin_size(origin, layout.box_size());
        self.text_constraints.insert(id, layout.constraints);
        self.push_text_arc(id, bounds, layout.props);
        id
    }

    fn push_text(&mut self, id: CanvasTextId, bounds: Bounds, props: TextProps) {
        self.push_text_arc(id, bounds, Arc::new(props));
    }

    fn push_text_arc(&mut self, id: CanvasTextId, bounds: Bounds, props: Arc<TextProps>) {
        self.commands
            .push(VectorCommand::TextBox { id, bounds, props });
    }

    /// Draws a GPU texture the application rendered itself.
    ///
    /// This is the wgpu path: render into a texture with your own pipeline --
    /// WGSL, compute, 3D, whatever the vector commands above cannot express --
    /// register it with the backend once, and place it here. The backend
    /// composites it with no copy, so it takes part in the surrounding drawing
    /// like any other image: clipped, transformed, blended, filtered.
    ///
    /// `id` must already be registered with the renderer (under `xui-skia`,
    /// `SkiaBackend::set_wgpu_texture`), and must stay stable across frames so
    /// the registration is reused. `revision` is what tells the renderer the
    /// *contents* changed -- bump it whenever you redraw into the texture, or
    /// the canvas will keep showing the frame it already painted.
    ///
    /// An unregistered id draws nothing rather than failing the frame: a
    /// texture that is not ready yet is a normal state during startup, and
    /// dropping a frame's worth of the whole canvas for it would be worse.
    pub fn texture(&mut self, id: ExternalTextureId, revision: u64, bounds: Bounds) -> &mut Self {
        self.texture_with(id, revision, bounds, 1.0, Sampling::Linear)
    }

    /// [`CanvasPainter::texture`] with an explicit opacity and sampling mode.
    pub fn texture_with(
        &mut self,
        id: ExternalTextureId,
        revision: u64,
        bounds: Bounds,
        opacity: f32,
        sampling: Sampling,
    ) -> &mut Self {
        self.commands.push(VectorCommand::Texture {
            id,
            // The caller registered this texture with the renderer itself.
            source: None,
            revision,
            bounds,
            opacity,
            sampling,
        });
        self
    }

    /// Records a region that [`CanvasController::pick`] can hit-test.
    ///
    /// Bounds-level, and last emitted wins, which is what a tooltip or a hover
    /// highlight needs. It is deliberately not exact path containment.
    pub fn pick(&mut self, tag: CanvasPickTag, bounds: Bounds) -> &mut Self {
        self.picks.push(CanvasPick { tag, bounds });
        self
    }
}

/// One run of adjacent commands that lower to a single render primitive.
///
/// Grouping is by *adjacency*, not by kind: authoring order is the paint order,
/// so a drawing that emits grid shapes, then a line path, then marker shapes,
/// then labels produces four primitives -- of which exactly one is a vector
/// run, the only expensive kind.
#[derive(Debug, Clone)]
enum CanvasBatch {
    Shapes(Vec<ShapePrimitive>),
    Vector(VectorScene),
    Text {
        id: CanvasTextId,
        bounds: Bounds,
        props: std::sync::Arc<TextProps>,
        constraints: TextLayoutConstraints,
    },
}

/// The lowered form of a canvas's content, rebuilt only when the content, the
/// measured size, or the style actually changes -- never per repaint.
#[derive(Debug, Clone, Default)]
struct CompiledCanvas {
    batches: Vec<CanvasBatch>,
    size: Size<f32>,
    revision: u64,
    /// Bumped on every compile so text primitives invalidate their shaping.
    generation: u64,
}

impl CompiledCanvas {
    fn text_boxes(
        &self,
    ) -> impl Iterator<Item = (CanvasTextId, Bounds, &TextProps, TextLayoutConstraints)> {
        self.batches.iter().filter_map(|batch| match batch {
            CanvasBatch::Text {
                id,
                bounds,
                props,
                constraints,
            } => Some((*id, *bounds, props.as_ref(), *constraints)),
            _ => None,
        })
    }
}

fn compile_commands(
    commands: &[VectorCommand],
    text_constraints: &HashMap<CanvasTextId, TextLayoutConstraints>,
) -> Vec<CanvasBatch> {
    let mut batches = Vec::new();
    let mut shapes: Vec<ShapePrimitive> = Vec::new();
    let mut vectors: Vec<VectorCommand> = Vec::new();

    fn flush_shapes(batches: &mut Vec<CanvasBatch>, shapes: &mut Vec<ShapePrimitive>) {
        if !shapes.is_empty() {
            batches.push(CanvasBatch::Shapes(std::mem::take(shapes)));
        }
    }
    fn flush_vectors(batches: &mut Vec<CanvasBatch>, vectors: &mut Vec<VectorCommand>) {
        if !vectors.is_empty() {
            batches.push(CanvasBatch::Vector(VectorScene::new(std::mem::take(
                vectors,
            ))));
        }
    }

    for command in commands {
        match command {
            VectorCommand::Shape {
                bounds,
                shape,
                fill,
                stroke,
            } => {
                flush_vectors(&mut batches, &mut vectors);
                shapes.push(ShapePrimitive {
                    bounds: *bounds,
                    shape: *shape,
                    fill: *fill,
                    stroke: *stroke,
                    shadow: None,
                });
            }
            // Textures join the vector run rather than opening a batch of
            // their own: they are drawn by the same backend pass as paths, and
            // keeping them in one run preserves the order a drawing was
            // emitted in without costing an extra flush.
            VectorCommand::FillPath { .. }
            | VectorCommand::StrokePath { .. }
            | VectorCommand::Texture { .. } => {
                flush_shapes(&mut batches, &mut shapes);
                vectors.push(command.clone());
            }
            VectorCommand::TextBox { id, bounds, props } => {
                flush_shapes(&mut batches, &mut shapes);
                flush_vectors(&mut batches, &mut vectors);
                batches.push(CanvasBatch::Text {
                    id: *id,
                    bounds: *bounds,
                    props: props.clone(),
                    constraints: text_constraints.get(id).copied().unwrap_or_else(|| {
                        TextLayoutConstraints::max_width(bounds.width().max(0.0))
                    }),
                });
            }
        }
    }
    flush_shapes(&mut batches, &mut shapes);
    flush_vectors(&mut batches, &mut vectors);
    batches
}

/// The queue a [`CanvasController`] drops invalidations into.
///
/// A canvas is a leaf: its content changes nothing about layout or the
/// component tree, so repainting one through a component rebuild -- the only
/// route a controller used to have -- pays for a full reconcile to move some
/// pixels. This channel marks the host node directly instead, and the runtime
/// reports the pending work through `is_dirty`, which is what gets the frame
/// scheduled.
#[derive(Clone, Default)]
pub(crate) struct CanvasInvalidator {
    pending: Rc<RefCell<Vec<(NodeId, WidgetUpdateFlags)>>>,
    wake: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
}

impl CanvasInvalidator {
    pub(crate) fn set_wake(&self, wake: Rc<dyn Fn()>) {
        *self.wake.borrow_mut() = Some(wake);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pending.borrow().is_empty()
    }

    pub(crate) fn drain(&self) -> Vec<(NodeId, WidgetUpdateFlags)> {
        std::mem::take(&mut *self.pending.borrow_mut())
    }

    fn push(&self, id: NodeId, flags: WidgetUpdateFlags) {
        self.pending.borrow_mut().push((id, flags));
        let wake = self.wake.borrow().clone();
        if let Some(wake) = wake {
            wake();
        }
    }
}

struct CanvasControllerState {
    content: CanvasContent,
    revision: u64,
    /// Every mounted node drawing this controller. Usually one; a controller
    /// shared by two canvases invalidates both.
    bindings: Vec<NodeId>,
    invalidator: Option<CanvasInvalidator>,
    picks: Vec<CanvasPick>,
    size: Size<f32>,
}

/// Shared retained content for a [`CanvasWidget`].
///
/// Mutating a controller repaints every canvas bound to it on the next frame,
/// with no component rebuild involved.
#[derive(Clone)]
pub struct CanvasController {
    inner: Rc<RefCell<CanvasControllerState>>,
}

impl fmt::Debug for CanvasController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.inner.borrow();
        f.debug_struct("CanvasController")
            .field("content", &state.content)
            .field("revision", &state.revision)
            .field("bindings", &state.bindings.len())
            .finish_non_exhaustive()
    }
}

impl CanvasController {
    pub fn new() -> Self {
        Self::with_content(CanvasContent::default())
    }

    pub fn with_scene(scene: VectorScene) -> Self {
        Self::with_content(CanvasContent::Scene(scene))
    }

    /// Builds a controller whose drawing is produced after layout, with the
    /// node's measured size.
    ///
    /// Hold this in a `use_ref` rather than rebuilding it each render: a new
    /// controller handle is a new identity, which forces a recompile.
    pub fn with_painter(painter: impl Fn(&mut CanvasPainter<'_>) + 'static) -> Self {
        Self::with_content(CanvasContent::Painter(Rc::new(painter)))
    }

    /// Builds a controller whose drawing is produced by the application's own
    /// GPU code, on the device the renderer runs on.
    ///
    /// The painter runs after layout with the node's measured size, again on
    /// resize and on [`CanvasController::invalidate`], and records a render
    /// pass into a target the framework allocated and composites afterwards.
    /// So a shader becomes a widget with no texture bookkeeping.
    ///
    /// It is handed `&mut Option<S>` -- state the canvas keeps between draws.
    /// Anything expensive belongs there, pipelines above all: creating one
    /// compiles a shader, which is milliseconds, and there is nothing about a
    /// pipeline that a resize invalidates. Only the render target is
    /// size-dependent, and the framework reallocates that.
    ///
    /// ```ignore
    /// CanvasController::with_gpu_painter(|state, gpu| {
    ///     let pipelines = state.get_or_insert_with(|| {
    ///         Pipelines::new(gpu.device(), gpu.format())
    ///     });
    ///
    ///     let mut encoder = gpu.device().create_command_encoder(&Default::default());
    ///     {
    ///         let mut pass = encoder.begin_render_pass(&/* gpu.target() */);
    ///         pass.set_pipeline(&pipelines.shader);
    ///         pass.draw(0..3, 0..1);
    ///     }
    ///     gpu.queue().submit([encoder.finish()]);
    /// })
    /// ```
    ///
    /// Because initialization happens inside the draw, it sees everything a
    /// draw sees -- the size, the style, the theme -- and rebuilding is just
    /// `*state = None`, which is what a format or theme change would need.
    ///
    /// Hold the controller in a `use_ref` rather than rebuilding it each
    /// render: a new controller is a new identity, which drops the state and
    /// reallocates the target.
    #[cfg(feature = "wgpu")]
    pub fn with_gpu_painter<S: 'static>(
        draw: impl Fn(&mut Option<S>, &mut CanvasGpuPainter<'_>) + 'static,
    ) -> Self {
        Self::with_content(CanvasContent::GpuPainter(Rc::new(GpuPainterFn {
            draw,
            state: RefCell::new(None),
        })))
    }

    pub fn with_content(content: CanvasContent) -> Self {
        Self {
            inner: Rc::new(RefCell::new(CanvasControllerState {
                content,
                revision: 1,
                bindings: Vec::new(),
                invalidator: None,
                picks: Vec::new(),
                size: Size::new(0.0, 0.0),
            })),
        }
    }

    pub fn content(&self) -> CanvasContent {
        self.inner.borrow().content.clone()
    }

    /// The retained scene, or `None` when this controller draws with a painter.
    pub fn scene(&self) -> Option<VectorScene> {
        match &self.inner.borrow().content {
            CanvasContent::Scene(scene) => Some(scene.clone()),
            CanvasContent::Painter(_) => None,
            #[cfg(feature = "wgpu")]
            CanvasContent::GpuPainter(_) => None,
        }
    }

    pub fn revision(&self) -> u64 {
        self.inner.borrow().revision
    }

    pub fn same_handle(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    pub fn set_scene(&self, scene: VectorScene) {
        let unchanged = matches!(
            &self.inner.borrow().content,
            CanvasContent::Scene(current) if *current == scene
        );
        if unchanged {
            return;
        }
        self.inner.borrow_mut().content = CanvasContent::Scene(scene);
        self.invalidate();
    }

    pub fn set_painter(&self, painter: impl Fn(&mut CanvasPainter<'_>) + 'static) {
        self.inner.borrow_mut().content = CanvasContent::Painter(Rc::new(painter));
        self.invalidate();
    }

    /// Edits the retained scene in place. Does nothing for painter content,
    /// which has no scene to edit -- call [`CanvasController::invalidate`] to
    /// re-run the painter instead.
    pub fn update(&self, update: impl FnOnce(&mut VectorScene)) {
        let next = {
            let mut state = self.inner.borrow_mut();
            let CanvasContent::Scene(scene) = &mut state.content else {
                return;
            };
            let previous = scene.clone();
            update(scene);
            *scene != previous
        };
        if next {
            self.invalidate();
        }
    }

    pub fn clear(&self) {
        self.set_scene(VectorScene::default());
    }

    /// Marks every canvas drawing this controller for a repaint, re-running the
    /// painter if there is one.
    pub fn invalidate(&self) {
        let (invalidator, bindings) = {
            let mut state = self.inner.borrow_mut();
            state.revision = state.revision.wrapping_add(1);
            (state.invalidator.clone(), state.bindings.clone())
        };
        let Some(invalidator) = invalidator else {
            return;
        };
        for id in bindings {
            invalidator.push(
                id,
                WidgetUpdateFlags::PAINT_OUTPUT | WidgetUpdateFlags::TEXT_SHAPE,
            );
        }
    }

    /// The size the drawing was last compiled for.
    pub fn size(&self) -> Size<f32> {
        self.inner.borrow().size
    }

    pub fn picks(&self) -> Vec<CanvasPick> {
        self.inner.borrow().picks.clone()
    }

    /// Hit-tests a point in canvas-local coordinates -- the `current_local`
    /// coordinate a pointer event on the canvas carries.
    ///
    /// The last region emitted that contains the point wins, matching paint
    /// order.
    pub fn pick(&self, point: Point) -> Option<CanvasPickTag> {
        self.inner
            .borrow()
            .picks
            .iter()
            .rev()
            .find(|pick| pick.bounds.contains(point))
            .map(|pick| pick.tag)
    }

    pub(crate) fn bind(&self, id: NodeId, invalidator: CanvasInvalidator) {
        let mut state = self.inner.borrow_mut();
        if !state.bindings.contains(&id) {
            state.bindings.push(id);
        }
        state.invalidator = Some(invalidator);
    }

    pub(crate) fn unbind(&self, id: NodeId) {
        let mut state = self.inner.borrow_mut();
        state.bindings.retain(|bound| *bound != id);
        if state.bindings.is_empty() {
            state.invalidator = None;
        }
    }

    fn publish_compiled(&self, size: Size<f32>, picks: Option<Vec<CanvasPick>>) {
        let mut state = self.inner.borrow_mut();
        state.size = size;
        if let Some(picks) = picks {
            state.picks = picks;
        }
    }

    fn identity(&self) -> usize {
        Rc::as_ptr(&self.inner) as usize
    }
}

impl Default for CanvasController {
    fn default() -> Self {
        Self::new()
    }
}

pub struct CanvasWidget {
    pub key: Option<Key>,
    pub controller: CanvasController,
    pub style: Style,
    pub event_handlers: EventHandlers,
    pub interaction: InteractionProperties,
    compiled: CompiledCanvas,
    /// The target a GPU painter draws into, kept across compiles and
    /// reallocated when the node's physical size changes.
    #[cfg(feature = "wgpu")]
    gpu_target: Option<GpuTarget>,
    /// Bumped every time a GPU painter runs. The texture id is stable so the
    /// renderer keeps one import, which means new contents are invisible to
    /// scene diffing without this.
    #[cfg(feature = "wgpu")]
    gpu_revision: u64,
    /// The host node this widget is mounted on, and the channel its controller
    /// invalidates through. Held here so that swapping the controller can move
    /// the binding without the pipeline having to know it happened.
    binding: Option<(NodeId, CanvasInvalidator)>,
}

/// A canvas GPU painter's render target.
#[cfg(feature = "wgpu")]
struct GpuTarget {
    id: ExternalTextureId,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    handle: ExternalTextureHandle,
    size: Size<u32>,
}

impl fmt::Debug for CanvasWidget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CanvasWidget")
            .field("key", &self.key)
            .field("controller", &self.controller)
            .field("style", &self.style)
            .field("batches", &self.compiled.batches.len())
            .finish()
    }
}

impl CanvasWidget {
    pub fn new(controller: CanvasController) -> Self {
        Self {
            key: None,
            controller,
            style: Style::new(),
            event_handlers: EventHandlers::default(),
            interaction: InteractionProperties::default(),
            compiled: CompiledCanvas::default(),
            #[cfg(feature = "wgpu")]
            gpu_target: None,
            #[cfg(feature = "wgpu")]
            gpu_revision: 0,
            binding: None,
        }
    }

    pub fn controller(mut self, controller: CanvasController) -> Self {
        self.controller = controller;
        self
    }

    /// Merges a style in, rather than replacing what is already there.
    ///
    /// On a fresh builder — how nearly every call site uses it — merging into an
    /// all-unset style is indistinguishable from assignment. The difference
    /// shows in the `xui!` macro, where `style={..}` is one attribute among
    /// many: assignment made `<column padding={..} style={..} />` silently
    /// discard the padding, and whether an attribute survived depended on
    /// whether it was written before or after `style`.
    pub fn style(mut self, style: impl xui_interface::StyleMerge) -> Self {
        self.style.merge(&style);
        self
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn into_element_desc(self) -> ElementDesc {
        widget_element_desc(self, Vec::new())
    }

    event_handler_methods!();
}

impl CanvasWidget {
    pub(super) fn node_type(&self) -> WidgetType {
        WidgetType::Canvas
    }

    pub(super) fn get_key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    /// Deliberately free of the content revision.
    ///
    /// Content changes reach the host through the invalidation channel, not
    /// through the props diff, so folding the revision in here would only make
    /// component rebuilds churn the hash for a repaint that was already
    /// scheduled.
    pub(super) fn props_hash(&self) -> u64 {
        props_hash(&(self.controller.identity(), &self.style))
    }

    pub(super) fn update_from(&mut self, next: &Self) -> WidgetUpdateFlags {
        let mut flags = WidgetUpdateFlags::empty();

        if !self.controller.same_handle(&next.controller) {
            let binding = self.binding.clone();
            if let Some((id, _)) = &binding {
                self.controller.unbind(*id);
            }
            self.controller = next.controller.clone();
            if let Some((id, invalidator)) = binding {
                self.controller.bind(id, invalidator);
            }
            self.compiled = CompiledCanvas::default();
            flags |= WidgetUpdateFlags::PAINT_OUTPUT | WidgetUpdateFlags::TEXT_SHAPE;
        }

        if self.style != next.style {
            self.style = next.style.clone();
            flags |= WidgetUpdateFlags::STYLE_TARGET;
        }

        flags
    }

    pub(super) fn default_style(&self) -> Style {
        Style::new()
    }

    pub(super) fn current_style(&self) -> &Style {
        &self.style
    }

    /// Re-runs the content against the node's final geometry.
    ///
    /// Called from the post-layout pass, next to the one that re-shapes text
    /// widgets at their committed width, because a painter needs exactly the
    /// same thing: a size Taffy has finished deciding.
    pub(crate) fn compile(
        &mut self,
        size: Size<f32>,
        style: &ComputedStyle,
        theme: &Theme,
        scale_factor: f32,
        measure_text: &mut dyn FnMut(
            CanvasTextId,
            &TextProps,
            TextLayoutConstraints,
        ) -> CanvasTextMetrics,
        gpu: Option<&CanvasGpuContext>,
    ) {
        let _ = &gpu;
        let content = self.controller.content();
        let revision = self.controller.revision();

        // A retained scene reads neither the size nor the style, so a resize or
        // a theme change must not rebuild it: rebuilding would mint new scene
        // ids and throw away every compiled path the backend has cached for
        // them, to arrive at exactly the same picture.
        if !content.is_size_dependent()
            && self.compiled.generation != 0
            && self.compiled.revision == revision
        {
            self.compiled.size = size;
            self.controller.publish_compiled(size, None);
            return;
        }

        let (batches, picks) = match content {
            CanvasContent::Scene(scene) => (
                compile_commands(scene.commands(), &HashMap::new()),
                Vec::new(),
            ),
            CanvasContent::Painter(painter) => {
                let mut painter_cx =
                    CanvasPainter::new(size, style, theme, scale_factor, measure_text);
                painter(&mut painter_cx);
                (
                    compile_commands(&painter_cx.commands, &painter_cx.text_constraints),
                    std::mem::take(&mut painter_cx.picks),
                )
            }
            // No device means the renderer is not on the shared-device path, so
            // there is nothing to draw with. An empty canvas is the right
            // answer: the rest of the UI is unaffected, and the painter runs as
            // soon as a device does arrive.
            #[cfg(feature = "wgpu")]
            CanvasContent::GpuPainter(_) if gpu.is_none() => (Vec::new(), Vec::new()),
            #[cfg(feature = "wgpu")]
            CanvasContent::GpuPainter(painter) => {
                let gpu = gpu.expect("guarded by the arm above");
                self.run_gpu_painter(&painter, gpu, size, style, theme, scale_factor)
            }
        };

        self.controller.publish_compiled(size, Some(picks));
        self.compiled = CompiledCanvas {
            batches,
            size,
            revision,
            generation: self.compiled.generation.wrapping_add(1),
        };
    }

    /// Allocates the target if needed, runs the painter, and lowers the result
    /// to the single texture command that draws it.
    #[cfg(feature = "wgpu")]
    fn run_gpu_painter(
        &mut self,
        painter: &Rc<dyn GpuPainter>,
        gpu: &CanvasGpuContext,
        size: Size<f32>,
        style: &ComputedStyle,
        theme: &Theme,
        scale_factor: f32,
    ) -> (Vec<CanvasBatch>, Vec<CanvasPick>) {
        // Physical pixels, rounded up so a fractional layout never loses a
        // column, and never zero -- wgpu rejects a zero-sized texture, and a
        // canvas can legitimately measure to nothing mid-layout.
        let physical = Size::new(
            ((size.width * scale_factor).ceil() as u32).max(1),
            ((size.height * scale_factor).ceil() as u32).max(1),
        );

        let stale = self
            .gpu_target
            .as_ref()
            .is_none_or(|target| target.size != physical);
        if stale {
            // The id outlives the texture: keeping it stable across resizes
            // means the renderer replaces one entry instead of leaking one per
            // size the window passed through.
            let id = self
                .gpu_target
                .as_ref()
                .map(|target| target.id)
                .unwrap_or_else(ExternalTextureId::next);
            let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
                label: Some("xui canvas gpu painter target"),
                size: wgpu::Extent3d {
                    width: physical.width,
                    height: physical.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: CANVAS_GPU_FORMAT,
                usage: CANVAS_GPU_USAGES,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.gpu_target = Some(GpuTarget {
                id,
                handle: ExternalTextureHandle::new(texture.clone()),
                texture,
                view,
                size: physical,
            });
        }
        let target = self.gpu_target.as_ref().expect("just allocated");

        let mut painter_cx = CanvasGpuPainter {
            device: gpu.device(),
            queue: gpu.queue(),
            target: &target.view,
            size: physical,
            logical_size: size,
            scale_factor,
            style,
            theme,
            picks: Vec::new(),
        };
        painter.draw(&mut painter_cx);
        let picks = std::mem::take(&mut painter_cx.picks);

        self.gpu_revision = self.gpu_revision.wrapping_add(1);
        let command = VectorCommand::Texture {
            id: target.id,
            source: Some(target.handle.clone()),
            revision: self.gpu_revision,
            bounds: Bounds::from_zero_size(size),
            opacity: 1.0,
            sampling: Sampling::Linear,
        };
        (compile_commands(&[command], &HashMap::new()), picks)
    }

    /// The texture a GPU painter last drew into, if any. Exposed for tests and
    /// for callers that want to sample the canvas's own output.
    #[cfg(feature = "wgpu")]
    pub fn gpu_texture(&self) -> Option<&wgpu::Texture> {
        self.gpu_target.as_ref().map(|target| &target.texture)
    }

    pub(crate) fn bind(&mut self, id: NodeId, invalidator: CanvasInvalidator) {
        self.controller.bind(id, invalidator.clone());
        self.binding = Some((id, invalidator));
    }

    pub(crate) fn unbind(&mut self) {
        if let Some((id, _)) = self.binding.take() {
            self.controller.unbind(id);
        }
    }

    pub(crate) fn text_boxes(
        &self,
    ) -> Vec<(CanvasTextId, Bounds, TextProps, TextLayoutConstraints)> {
        self.compiled
            .text_boxes()
            .map(|(id, bounds, props, constraints)| (id, bounds, props.clone(), constraints))
            .collect()
    }

    pub(super) fn render(
        &self,
        node_id: xui_interface::NodeId,
        rect: Bounds,
        _style: &ComputedStyle,
        writer: &mut RenderTreeWriter<'_>,
    ) {
        let origin = rect.origin();
        writer
            .clip(ClipShape::Rect(rect), |writer| {
                for batch in &self.compiled.batches {
                    match batch {
                        CanvasBatch::Shapes(shapes) => {
                            for shape in shapes {
                                writer.primitive(Primitive::Shape(ShapePrimitive {
                                    bounds: shape.bounds.translate(origin),
                                    // A line carries its endpoints in the same
                                    // space as the bounds, so they move too.
                                    shape: translate_shape(shape.shape, origin),
                                    ..*shape
                                }))?;
                            }
                        }
                        // The scene was built once at compile time, so its id
                        // -- and with it every compiled path the backend has
                        // cached for it -- survives this repaint.
                        CanvasBatch::Vector(scene) => {
                            writer.primitive(Primitive::Vector(VectorPrimitive {
                                scene: scene.clone(),
                                transform: Affine::translate(rect.x(), rect.y()),
                            }))?;
                        }
                        CanvasBatch::Text {
                            id, bounds, props, ..
                        } => {
                            let bounds = bounds.translate(origin);
                            let paint = TextPaintProps::new(TextPaintStyle {
                                color: props.style.color,
                                font_size: props.style.font_size,
                                line_height: props.style.line_height,
                                decoration: props.style.decoration,
                            });
                            writer.clip(ClipShape::Rect(bounds), |writer| {
                                writer.primitive(Primitive::Text(TextPrimitive {
                                    node_id,
                                    bounds,
                                    slot: canvas_text_slot(*id),
                                    layout_revision: self.compiled.generation,
                                    vertical_align: props.paragraph.vertical_align,
                                    paint,
                                }))?;
                                Ok(())
                            })?;
                        }
                    }
                }
                Ok(())
            })
            .expect("widget render tree must remain valid");
    }

    pub(super) fn handle_event(
        &mut self,
        _event: EventRef<'_>,
        _cx: &mut EventContext<'_>,
    ) -> EventResult {
        EventResult::Ignored
    }

    pub(super) fn text_content(&self) -> Option<TextContent> {
        None
    }

    pub(super) fn text_layout_props(&self, _style: &ComputedStyle) -> Option<TextProps> {
        None
    }
}

fn translate_shape(shape: Shape, origin: Point) -> Shape {
    match shape {
        Shape::Line { from, to } => Shape::Line {
            from: Point::new(from.x + origin.x, from.y + origin.y),
            to: Point::new(to.x + origin.x, to.y + origin.y),
        },
        other => other,
    }
}

pub(crate) fn canvas_text_slot(id: CanvasTextId) -> TextLayoutSlot {
    TextLayoutSlot::new(id.get())
}

#[cfg(test)]
impl CanvasWidget {
    pub(crate) fn compiled_generation(&self) -> u64 {
        self.compiled.generation
    }

    pub(crate) fn compiled_size(&self) -> Size<f32> {
        self.compiled.size
    }

    fn batch_kinds(&self) -> Vec<&'static str> {
        self.compiled
            .batches
            .iter()
            .map(|batch| match batch {
                CanvasBatch::Shapes(_) => "shapes",
                CanvasBatch::Vector(_) => "vector",
                CanvasBatch::Text { .. } => "text",
            })
            .collect()
    }

    fn vector_scene_ids(&self) -> Vec<xui_interface::VectorSceneId> {
        self.compiled
            .batches
            .iter()
            .filter_map(|batch| match batch {
                CanvasBatch::Vector(scene) => Some(scene.id()),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{RenderNodeKind, RenderScene};
    use crate::style::Theme as StyleTheme;
    use slotmap::Key as _;
    use xui_interface::{PathBuilder, VectorSceneBuilder};

    fn line_path() -> PathData {
        let mut path = PathBuilder::new();
        path.move_to(Point::new(0.0, 0.0))
            .line_to(Point::new(10.0, 10.0));
        path.build()
    }

    fn compile(widget: &mut CanvasWidget, size: Size<f32>) {
        let theme = StyleTheme::default();
        let style = ComputedStyle::initial(&theme);
        let mut measure_text = |_, _: &TextProps, _| CanvasTextMetrics {
            size: Size::<f32>::ZERO,
            first_baseline: None,
            line_count: 0,
        };
        widget.compile(size, &style, &theme, 2.0, &mut measure_text, None);
    }

    #[test]
    fn adjacent_commands_of_one_kind_lower_to_a_single_primitive_batch() {
        // The order a chart naturally emits in: grid, series, markers, labels.
        let controller = CanvasController::with_painter(|p| {
            p.line(
                Point::new(0.0, 20.0),
                Point::new(100.0, 20.0),
                Color::BLACK,
                1.0,
            );
            p.line(
                Point::new(0.0, 40.0),
                Point::new(100.0, 40.0),
                Color::BLACK,
                1.0,
            );
            p.stroke_path(line_path(), PathStroke::new(Color::BLACK, 2.0));
            p.circle(Point::new(10.0, 20.0), 3.0, Color::WHITE);
            p.circle(Point::new(40.0, 40.0), 3.0, Color::WHITE);
            p.text(
                Bounds::from_origin_size((0.0, 80.0), (60.0, 16.0)),
                TextProps::new("label"),
            );
        });
        let mut widget = CanvasWidget::new(controller);
        compile(&mut widget, Size::new(100.0, 100.0));

        assert_eq!(
            widget.batch_kinds(),
            vec!["shapes", "vector", "shapes", "text"],
            "only one vector run, which is the only expensive kind"
        );
    }

    #[test]
    fn interleaving_kinds_is_what_costs_extra_vector_runs() {
        let controller = CanvasController::with_painter(|p| {
            p.stroke_path(line_path(), PathStroke::new(Color::BLACK, 1.0));
            p.circle(Point::new(1.0, 1.0), 1.0, Color::WHITE);
            p.stroke_path(line_path(), PathStroke::new(Color::WHITE, 1.0));
        });
        let mut widget = CanvasWidget::new(controller);
        compile(&mut widget, Size::new(100.0, 100.0));
        assert_eq!(widget.batch_kinds(), vec!["vector", "shapes", "vector"]);
    }

    #[test]
    fn a_repaint_reuses_the_scene_built_at_compile_time() {
        let controller = CanvasController::with_painter(|p| {
            p.stroke_path(line_path(), PathStroke::new(Color::BLACK, 1.0));
        });
        let mut widget = CanvasWidget::new(controller);
        compile(&mut widget, Size::new(100.0, 100.0));
        let scenes = widget.vector_scene_ids();

        let mut render_scene = RenderScene::new();
        let parent = render_scene.insert_group();
        let theme = StyleTheme::default();
        let style = ComputedStyle::initial(&theme);
        let rect = Bounds::from_origin_size((5.0, 7.0), (100.0, 100.0));
        for _ in 0..3 {
            let mut writer = RenderTreeWriter::new(&mut render_scene, parent);
            widget.render(xui_interface::NodeId::null(), rect, &style, &mut writer);
            writer.finish().unwrap();
        }

        assert_eq!(
            widget.vector_scene_ids(),
            scenes,
            "painting must not mint a new scene id, or every backend path cache misses"
        );
    }

    #[test]
    fn a_painter_draws_against_the_measured_size() {
        let controller = CanvasController::with_painter(|p| {
            let width = p.width();
            p.rect(
                Bounds::from_origin_size((0.0, 0.0), (width, 4.0)),
                Color::BLACK,
            );
        });
        let mut widget = CanvasWidget::new(controller);

        compile(&mut widget, Size::new(300.0, 50.0));
        let full_width = |widget: &CanvasWidget| match &widget.compiled.batches[0] {
            CanvasBatch::Shapes(shapes) => shapes[0].bounds.width(),
            other => panic!("expected a shape batch, got {other:?}"),
        };
        assert_eq!(full_width(&widget), 300.0);

        compile(&mut widget, Size::new(520.0, 50.0));
        assert_eq!(full_width(&widget), 520.0);
        assert_eq!(widget.compiled_size(), Size::new(520.0, 50.0));
    }

    #[test]
    fn scale_factor_reaches_the_painter_for_hairlines_and_snapping() {
        let recorded = Rc::new(RefCell::new((0.0, 0.0)));
        let sink = recorded.clone();
        let controller = CanvasController::with_painter(move |p| {
            *sink.borrow_mut() = (p.hairline(), p.snap(10.3));
        });
        let mut widget = CanvasWidget::new(controller);
        compile(&mut widget, Size::new(100.0, 100.0));
        assert_eq!(*recorded.borrow(), (0.5, 10.5));
    }

    #[test]
    fn text_ids_are_stable_by_emission_order_and_keys_cannot_collide_with_them() {
        let controller = CanvasController::with_painter(|p| {
            let bounds = Bounds::from_origin_size((0.0, 0.0), (40.0, 12.0));
            p.text(bounds, TextProps::new("first"));
            p.text(bounds, TextProps::new("second"));
            p.text_keyed(1, bounds, TextProps::new("keyed"));
        });
        let mut widget = CanvasWidget::new(controller);

        compile(&mut widget, Size::new(100.0, 100.0));
        let first: Vec<_> = widget
            .text_boxes()
            .into_iter()
            .map(|(id, _, _, _)| id.get())
            .collect();
        assert_eq!(first, vec![1, 2, KEYED_TEXT_ID_BIT | 1]);

        compile(&mut widget, Size::new(100.0, 100.0));
        let second: Vec<_> = widget
            .text_boxes()
            .into_iter()
            .map(|(id, _, _, _)| id.get())
            .collect();
        assert_eq!(first, second, "a recompile must not move shaping slots");
    }

    #[test]
    fn laid_out_text_uses_measured_geometry_and_preserves_its_constraint() {
        let observed = Rc::new(RefCell::new(None));
        let sink = observed.clone();
        let controller = CanvasController::with_painter(move |p| {
            let layout = p.layout_text_keyed(
                7,
                TextProps::new("measured"),
                TextLayoutConstraints::max_width(80.0),
            );
            *sink.borrow_mut() = Some(layout.metrics());
            p.draw_text(Point::new(12.0, 20.0), layout);
        });
        let mut widget = CanvasWidget::new(controller);
        let theme = StyleTheme::default();
        let style = ComputedStyle::initial(&theme);
        let mut measure_text =
            |id: CanvasTextId, props: &TextProps, constraints: TextLayoutConstraints| {
                assert_eq!(id, CanvasTextId::new(KEYED_TEXT_ID_BIT | 7));
                assert_eq!(props.text.as_str(), "measured");
                assert_eq!(constraints, TextLayoutConstraints::max_width(80.0));
                CanvasTextMetrics {
                    size: Size::new(54.0, 18.0),
                    first_baseline: Some(13.0),
                    line_count: 1,
                }
            };

        widget.compile(
            Size::new(200.0, 100.0),
            &style,
            &theme,
            2.0,
            &mut measure_text,
            None,
        );

        assert_eq!(
            *observed.borrow(),
            Some(CanvasTextMetrics {
                size: Size::new(54.0, 18.0),
                first_baseline: Some(13.0),
                line_count: 1,
            })
        );
        let text_boxes = widget.text_boxes();
        let [(id, bounds, _, constraints)] = text_boxes.as_slice() else {
            panic!("expected exactly one laid-out text box");
        };
        assert_eq!(*id, CanvasTextId::new(KEYED_TEXT_ID_BIT | 7));
        assert_eq!(
            *bounds,
            Bounds::from_origin_size((12.0, 20.0), (80.0, 18.0))
        );
        assert_eq!(*constraints, TextLayoutConstraints::max_width(80.0));
    }

    #[test]
    fn picks_are_published_to_the_controller_with_paint_order_on_top() {
        let controller = CanvasController::with_painter(|p| {
            p.pick(7, Bounds::from_origin_size((0.0, 0.0), (100.0, 100.0)));
            p.pick(9, Bounds::from_origin_size((10.0, 10.0), (20.0, 20.0)));
        });
        let mut widget = CanvasWidget::new(controller.clone());
        compile(&mut widget, Size::new(100.0, 100.0));

        assert_eq!(controller.pick(Point::new(15.0, 15.0)), Some(9));
        assert_eq!(controller.pick(Point::new(80.0, 80.0)), Some(7));
        assert_eq!(controller.pick(Point::new(-1.0, 0.0)), None);
        assert_eq!(controller.size(), Size::new(100.0, 100.0));
    }

    #[test]
    fn editing_a_scene_bumps_the_revision_only_when_the_drawing_changed() {
        let scene = |color: Color| {
            let mut scene = VectorSceneBuilder::new();
            scene.fill_path(line_path(), Affine::IDENTITY, PathFill::new(color));
            scene.build()
        };

        let controller = CanvasController::with_scene(scene(Color::BLACK));
        let start = controller.revision();

        controller.set_scene(scene(Color::BLACK));
        assert_eq!(
            controller.revision(),
            start,
            "an identical scene is not a change, now that paths are content addressed"
        );

        controller.set_scene(scene(Color::WHITE));
        assert_eq!(controller.revision(), start + 1);

        controller.update(|scene| *scene = VectorScene::default());
        assert_eq!(controller.revision(), start + 2);
        controller.clear();
        assert_eq!(controller.revision(), start + 2);
    }

    #[test]
    fn update_does_nothing_to_painter_content_but_invalidate_still_redraws_it() {
        let runs = Rc::new(RefCell::new(0));
        let counter = runs.clone();
        let controller = CanvasController::with_painter(move |_| {
            *counter.borrow_mut() += 1;
        });
        let mut widget = CanvasWidget::new(controller.clone());
        compile(&mut widget, Size::new(10.0, 10.0));
        assert_eq!(*runs.borrow(), 1);

        let before = controller.revision();
        controller.update(|scene| *scene = VectorScene::default());
        assert_eq!(controller.revision(), before, "there is no scene to edit");

        controller.invalidate();
        assert_eq!(controller.revision(), before + 1);
        compile(&mut widget, Size::new(10.0, 10.0));
        assert_eq!(*runs.borrow(), 2);
    }

    #[test]
    fn the_props_hash_ignores_content_so_edits_do_not_churn_the_component_diff() {
        let controller = CanvasController::with_scene(VectorScene::default());
        let widget = CanvasWidget::new(controller.clone());
        let before = widget.props_hash();

        controller.set_scene({
            let mut scene = VectorSceneBuilder::new();
            scene.fill_path(line_path(), Affine::IDENTITY, PathFill::new(Color::BLACK));
            scene.build()
        });
        assert_eq!(
            widget.props_hash(),
            before,
            "content reaches the host through the invalidation channel, not the props diff"
        );

        let replacement = CanvasWidget::new(CanvasController::new());
        assert_ne!(replacement.props_hash(), before);
    }

    #[test]
    fn swapping_the_controller_moves_the_host_binding_with_it() {
        let invalidator = CanvasInvalidator::default();
        let node = xui_interface::NodeId::null();

        let first = CanvasController::new();
        let second = CanvasController::new();
        let mut widget = CanvasWidget::new(first.clone());
        widget.bind(node, invalidator.clone());

        let flags = widget.update_from(&CanvasWidget::new(second.clone()));
        assert!(flags.contains(WidgetUpdateFlags::PAINT_OUTPUT));

        first.invalidate();
        assert!(
            invalidator.is_empty(),
            "the replaced controller must stop marking a node it no longer draws"
        );

        second.invalidate();
        assert_eq!(invalidator.drain().len(), 1);
    }

    #[test]
    fn a_retained_scene_survives_a_resize_without_being_rebuilt() {
        let mut scene = VectorSceneBuilder::new();
        scene.stroke_path(
            line_path(),
            Affine::IDENTITY,
            PathStroke::new(Color::BLACK, 1.0),
        );
        let controller = CanvasController::with_scene(scene.build());
        let mut widget = CanvasWidget::new(controller.clone());

        compile(&mut widget, Size::new(100.0, 50.0));
        let generation = widget.compiled_generation();
        let scenes = widget.vector_scene_ids();

        compile(&mut widget, Size::new(400.0, 50.0));
        assert_eq!(
            widget.compiled_generation(),
            generation,
            "a scene does not read the size, so a resize is not a reason to rebuild it"
        );
        assert_eq!(widget.vector_scene_ids(), scenes);
        assert_eq!(controller.size(), Size::new(400.0, 50.0));

        let mut next = VectorSceneBuilder::new();
        next.stroke_path(
            line_path(),
            Affine::IDENTITY,
            PathStroke::new(Color::WHITE, 1.0),
        );
        controller.set_scene(next.build());
        compile(&mut widget, Size::new(400.0, 50.0));
        assert_eq!(widget.compiled_generation(), generation + 1);
    }

    #[test]
    fn a_line_moves_its_endpoints_with_the_node_not_just_its_bounds() {
        let controller = CanvasController::with_painter(|p| {
            p.line(
                Point::new(0.0, 10.0),
                Point::new(100.0, 10.0),
                Color::BLACK,
                1.0,
            );
        });
        let mut widget = CanvasWidget::new(controller);
        compile(&mut widget, Size::new(100.0, 50.0));

        let mut render_scene = RenderScene::new();
        let parent = render_scene.insert_group();
        let theme = StyleTheme::default();
        let style = ComputedStyle::initial(&theme);
        let rect = Bounds::from_origin_size((20.0, 30.0), (100.0, 50.0));

        let mut writer = RenderTreeWriter::new(&mut render_scene, parent);
        widget.render(xui_interface::NodeId::null(), rect, &style, &mut writer);
        writer.finish().unwrap();

        let outer_clip = render_scene.children(parent).unwrap()[0];
        let RenderNodeKind::Clip(outer) = &render_scene.node(outer_clip).unwrap().kind else {
            panic!("canvas root should be a clip");
        };
        let child = render_scene.children(outer.child.unwrap()).unwrap()[0];
        let RenderNodeKind::Primitive(node) = &render_scene.node(child).unwrap().kind else {
            panic!("expected a primitive");
        };
        let Primitive::Shape(shape) = &node.primitive else {
            panic!("expected a shape");
        };
        // Both backends read the endpoints in the same space as the bounds, so
        // leaving them at canvas-local coordinates draws the rule off-node.
        assert_eq!(
            shape.shape,
            Shape::Line {
                from: Point::new(20.0, 40.0),
                to: Point::new(120.0, 40.0),
            }
        );
    }

    #[test]
    fn render_clips_to_the_node_and_places_every_batch_at_its_origin() {
        let controller = CanvasController::with_painter(|p| {
            p.rect(
                Bounds::from_origin_size((0.0, 0.0), (10.0, 10.0)),
                Color::BLACK,
            );
            p.stroke_path(line_path(), PathStroke::new(Color::WHITE, 1.0));
            p.text(
                Bounds::from_origin_size((10.0, 12.0), (80.0, 24.0)),
                TextProps::new("canvas"),
            );
        });
        let mut widget = CanvasWidget::new(controller);
        compile(&mut widget, Size::new(100.0, 50.0));

        let mut render_scene = RenderScene::new();
        let parent = render_scene.insert_group();
        let theme = StyleTheme::default();
        let style = ComputedStyle::initial(&theme);
        let rect = Bounds::from_origin_size((2.0, 3.0), (100.0, 50.0));

        let mut writer = RenderTreeWriter::new(&mut render_scene, parent);
        widget.render(xui_interface::NodeId::null(), rect, &style, &mut writer);
        writer.finish().unwrap();

        let outer_clip = render_scene.children(parent).unwrap()[0];
        let RenderNodeKind::Clip(outer) = &render_scene.node(outer_clip).unwrap().kind else {
            panic!("canvas root should be a clip");
        };
        assert_eq!(outer.clip, ClipShape::Rect(rect));

        let children = render_scene
            .children(outer.child.unwrap())
            .unwrap()
            .to_vec();
        assert_eq!(children.len(), 3);

        let RenderNodeKind::Primitive(shape) = &render_scene.node(children[0]).unwrap().kind else {
            panic!("first batch should be a shape primitive");
        };
        let Primitive::Shape(shape) = &shape.primitive else {
            panic!("first batch should be a shape");
        };
        assert_eq!(
            shape.bounds,
            Bounds::from_origin_size((2.0, 3.0), (10.0, 10.0))
        );

        let RenderNodeKind::Primitive(vector) = &render_scene.node(children[1]).unwrap().kind
        else {
            panic!("second batch should be a vector primitive");
        };
        let Primitive::Vector(vector) = &vector.primitive else {
            panic!("second batch should be a vector run");
        };
        assert_eq!(vector.transform, Affine::translate(rect.x(), rect.y()));

        let RenderNodeKind::Clip(text_clip) = &render_scene.node(children[2]).unwrap().kind else {
            panic!("a text box introduces its own clip");
        };
        let text_node = render_scene.children(text_clip.child.unwrap()).unwrap()[0];
        let RenderNodeKind::Primitive(text) = &render_scene.node(text_node).unwrap().kind else {
            panic!("text clip should contain a primitive");
        };
        let Primitive::Text(text) = &text.primitive else {
            panic!("third batch should be text");
        };
        assert_eq!(text.slot, TextLayoutSlot::new(1));
        assert_eq!(
            text.bounds,
            Bounds::from_origin_size((12.0, 15.0), (80.0, 24.0))
        );
    }

    /// The GPU painter path, on a real device.
    ///
    /// Covers the three invariants the design rests on: the painter gets a
    /// usable device and a target sized to the node, redrawing reuses the same
    /// texture (so the renderer keeps one import), and a resize reallocates it.
    #[cfg(feature = "wgpu")]
    mod gpu {
        use super::*;
        use std::cell::Cell;

        fn context() -> Option<CanvasGpuContext> {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter =
                pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: None,
                    force_fallback_adapter: false,
                }))
                .ok()?;
            let (device, queue) =
                pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                    label: Some("xui canvas gpu test"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                    ..Default::default()
                }))
                .ok()?;
            Some(CanvasGpuContext::new(device, queue))
        }

        fn compile_gpu(widget: &mut CanvasWidget, size: Size<f32>, gpu: &CanvasGpuContext) {
            let theme = StyleTheme::default();
            let style = ComputedStyle::initial(&theme);
            let mut measure_text = |_, _: &TextProps, _| CanvasTextMetrics {
                size: Size::<f32>::ZERO,
                first_baseline: None,
                line_count: 0,
            };
            widget.compile(size, &style, &theme, 2.0, &mut measure_text, Some(gpu));
        }

        /// The single texture command a GPU painter lowers to.
        fn texture_command(widget: &CanvasWidget) -> VectorCommand {
            let batch = widget
                .compiled
                .batches
                .first()
                .expect("a gpu painter lowers to one batch");
            let CanvasBatch::Vector(scene) = batch else {
                panic!("a gpu painter lowers to a vector batch");
            };
            scene
                .commands()
                .first()
                .expect("one command in the batch")
                .clone()
        }

        #[test]
        fn a_gpu_painter_draws_into_a_target_sized_to_the_node() {
            let Some(gpu) = context() else {
                eprintln!("no wgpu adapter available, skipping");
                return;
            };
            let seen = Rc::new(Cell::new(None));
            let recorded = Rc::clone(&seen);
            let mut widget = CanvasWidget::new(CanvasController::with_gpu_painter(
                move |_state: &mut Option<()>, painter: &mut CanvasGpuPainter<'_>| {
                    recorded.set(Some(painter.size()));
                    // A real pipeline would render here; clearing is enough to
                    // prove the target and queue are usable.
                    let mut encoder = painter
                        .device()
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
                    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: None,
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: painter.target(),
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLUE),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    painter.queue().submit([encoder.finish()]);
                },
            ));

            compile_gpu(&mut widget, Size::new(30.0, 20.0), &gpu);

            // Logical size times the scale factor, in physical pixels.
            assert_eq!(seen.get(), Some(Size::new(60, 40)));
            let texture = widget.gpu_texture().expect("a target was allocated");
            assert_eq!((texture.width(), texture.height()), (60, 40));

            let VectorCommand::Texture { source, bounds, .. } = texture_command(&widget) else {
                panic!("a gpu painter lowers to a texture command");
            };
            assert!(
                source.is_some(),
                "the framework owns this target, so it travels with the command"
            );
            // Composited at the node's logical size, not the target's.
            assert_eq!(bounds.size(), Size::new(30.0, 20.0));
        }

        #[test]
        fn redrawing_reuses_the_texture_and_resizing_replaces_it() {
            let Some(gpu) = context() else {
                eprintln!("no wgpu adapter available, skipping");
                return;
            };
            let mut widget = CanvasWidget::new(CanvasController::with_gpu_painter(
                |_state: &mut Option<()>, _painter: &mut CanvasGpuPainter<'_>| {},
            ));

            compile_gpu(&mut widget, Size::new(10.0, 10.0), &gpu);
            let VectorCommand::Texture {
                id: first_id,
                source: first_source,
                revision: first_revision,
                ..
            } = texture_command(&widget)
            else {
                panic!("expected a texture command");
            };

            // Same size: the painter runs again, but into the same texture.
            compile_gpu(&mut widget, Size::new(10.0, 10.0), &gpu);
            let VectorCommand::Texture {
                id: second_id,
                source: second_source,
                revision: second_revision,
                ..
            } = texture_command(&widget)
            else {
                panic!("expected a texture command");
            };
            assert_eq!(first_id, second_id, "the id is stable across redraws");
            assert_eq!(
                first_source, second_source,
                "the same texture, so the renderer keeps its import"
            );
            assert_ne!(
                first_revision, second_revision,
                "new contents have to be visible to scene diffing"
            );

            // A resize reallocates, which the renderer must notice.
            compile_gpu(&mut widget, Size::new(20.0, 10.0), &gpu);
            let VectorCommand::Texture {
                id: third_id,
                source: third_source,
                ..
            } = texture_command(&widget)
            else {
                panic!("expected a texture command");
            };
            assert_eq!(
                first_id, third_id,
                "the id outlives the texture, so the renderer replaces one entry"
            );
            assert_ne!(
                second_source, third_source,
                "a new texture object must re-import"
            );
        }

        /// The reason the canvas keeps state at all: creating a pipeline
        /// compiles a shader, so it must happen once, not once a frame -- and
        /// a resize, which reallocates the target, must not disturb it.
        #[test]
        fn state_survives_redraws_and_resizes() {
            let Some(gpu) = context() else {
                eprintln!("no wgpu adapter available, skipping");
                return;
            };
            let inits = Rc::new(Cell::new(0u32));
            let draws = Rc::new(Cell::new(0u32));
            let counted = Rc::clone(&inits);
            let drawn = Rc::clone(&draws);
            let mut widget = CanvasWidget::new(CanvasController::with_gpu_painter(
                move |state: &mut Option<u32>, _painter: &mut CanvasGpuPainter<'_>| {
                    // Stands in for a pipeline: built once, reached every draw.
                    let built = state.get_or_insert_with(|| {
                        counted.set(counted.get() + 1);
                        7
                    });
                    assert_eq!(*built, 7, "the same state reaches every draw");
                    drawn.set(drawn.get() + 1);
                },
            ));

            compile_gpu(&mut widget, Size::new(10.0, 10.0), &gpu);
            compile_gpu(&mut widget, Size::new(10.0, 10.0), &gpu);
            // A resize reallocates the target, but nothing about a pipeline
            // depends on size, so the state has to come through untouched.
            compile_gpu(&mut widget, Size::new(20.0, 10.0), &gpu);

            assert_eq!(inits.get(), 1, "state must not be rebuilt per draw");
            assert_eq!(draws.get(), 3);
        }

        /// `*state = None` is the whole rebuild story -- what a format or theme
        /// change would use.
        #[test]
        fn clearing_the_state_rebuilds_it() {
            let Some(gpu) = context() else {
                eprintln!("no wgpu adapter available, skipping");
                return;
            };
            let inits = Rc::new(Cell::new(0u32));
            let counted = Rc::clone(&inits);
            let clear_next = Rc::new(Cell::new(false));
            let clear = Rc::clone(&clear_next);
            let mut widget = CanvasWidget::new(CanvasController::with_gpu_painter(
                move |state: &mut Option<u32>, _painter: &mut CanvasGpuPainter<'_>| {
                    if clear.replace(false) {
                        *state = None;
                    }
                    state.get_or_insert_with(|| {
                        counted.set(counted.get() + 1);
                        7
                    });
                },
            ));

            compile_gpu(&mut widget, Size::new(10.0, 10.0), &gpu);
            assert_eq!(inits.get(), 1);
            clear_next.set(true);
            compile_gpu(&mut widget, Size::new(10.0, 10.0), &gpu);
            assert_eq!(inits.get(), 2, "clearing the state rebuilds it");
        }

        #[test]
        fn without_a_device_a_gpu_painter_draws_nothing() {
            let ran = Rc::new(Cell::new(false));
            let recorded = Rc::clone(&ran);
            let mut widget = CanvasWidget::new(CanvasController::with_gpu_painter(
                move |_state: &mut Option<()>, _painter: &mut CanvasGpuPainter<'_>| {
                    recorded.set(true);
                },
            ));
            compile(&mut widget, Size::new(10.0, 10.0));
            assert!(!ran.get(), "there is nothing to draw with");
            assert!(widget.compiled.batches.is_empty());
        }
    }
}
