use crate::{
    BlendMode, ColorMatrix, CompositeInstance, CompositeOperator, ExternalResourceKind,
    LayerProgram, MaskProgram, MaskShape, PlanError, ProgramOp, ProgramResourceKind,
    SampleExpansion,
};
use xui_interface::{Affine, Color, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PassId(u32);
impl PassId {
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlanResourceId(u32);
impl PlanResourceId {
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TransientSlotId(u32);
impl TransientSlotId {
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextureClass(pub u32);
impl TextureClass {
    pub const LINEAR_COLOR: Self = Self(0);
    pub const MASK: Self = Self(1);
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Extent2d {
    pub width: u32,
    pub height: u32,
}
impl Extent2d {
    pub const fn texels(self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PixelRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl PixelRect {
    pub const fn extent(self) -> Extent2d {
        Extent2d {
            width: self.width,
            height: self.height,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ExternalAliasing {
    #[default]
    Distinct,
    BackdropAndDestination,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlanLimits {
    pub max_texture_dimension_2d: u32,
}
impl Default for PlanLimits {
    fn default() -> Self {
        Self {
            max_texture_dimension_2d: 16_384,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerPlanContext {
    pub backdrop_source_bounds: Rect,
    /// Complete logical domain represented by the parent attachment.
    pub parent_destination_bounds: Rect,
    /// Optional placement/clip restriction inside the parent attachment.
    pub composite_clip_bounds: Option<Rect>,
    pub layer_content_bounds: Rect,
    pub backdrop_bounds: Option<Rect>,
    pub composite: CompositeInstance,
    pub scale_factor: f32,
    pub color_texture_class: TextureClass,
    pub external_aliasing: ExternalAliasing,
    pub limits: PlanLimits,
}

/// Selects which executable branch of a compiled layer program is lowered.
///
/// A complete layer first evaluates its backdrop branch and then evaluates the
/// layer-content/effect branch. Prefix materialization can request the backdrop
/// branch independently without relying on dynamic opacity to prune unrelated
/// work.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum LayerProgramEntry {
    #[default]
    Complete,
    BackdropOnly,
    LayerOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoordinateSpace {
    Parent,
    LayerLocal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    X,
    Y,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlanMask {
    None,
    Shape {
        shape: MaskShape,
        transform: Affine,
    },
    Texture {
        transform: Affine,
        resource: PlanResourceId,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum PassOp {
    Copy,
    GaussianBlur {
        axis: Axis,
        sigma_px: f32,
        support: f32,
    },
    ColorMatrix(ColorMatrix),
    Pixelate {
        block_width_px: f32,
        block_height_px: f32,
    },
    Refraction {
        strength_px: f32,
        chromatic_aberration_px: f32,
    },
    ChromaticAberration {
        offset_px: [f32; 2],
    },
    ExtractAlpha,
    AlphaSpread {
        axis: Axis,
        radius_px: f32,
    },
    ShadowComposite {
        color: Color,
        offset_px: [f32; 2],
    },
    ApplyMask {
        transform: Affine,
        mask: PlanResourceId,
    },
    BackdropComposite {
        opacity: f32,
        blend_mode: BlendMode,
        mask: PlanMask,
        bounds: Rect,
    },
    LayerComposite {
        opacity: f32,
        transform: Affine,
        blend_mode: BlendMode,
        operator: CompositeOperator,
        bounds: Rect,
    },
}

/// Backend-independent render-pipeline identity for a lowered pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipelineKey {
    Filter,
    Composite(AttachmentBlend),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AttachmentBlend {
    Replace,
    SrcOver,
    DstOver,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DrawShader {
    Filter,
    Composite,
    AttachmentBackdrop,
    AttachmentLayer,
}

/// Fully determined draw geometry and shader mode for one pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DrawProgram {
    pub shader: DrawShader,
    pub viewport: PixelRect,
    pub scissor: PixelRect,
    pub vertex_count: u32,
}

/// Fixed texture slots consumed by the executor's generic pass layout.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ResourceBindings {
    pub texture0: Option<PlanResourceId>,
    pub texture1: Option<PlanResourceId>,
    pub texture2: Option<PlanResourceId>,
}

/// Backend-independent values shared by every shader invocation. Operation-
/// specific values remain strongly typed in [`PassOp`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PassUniforms {
    pub output_logical_bounds: Rect,
    pub output_physical_bounds: PixelRect,
    pub scale_factor: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pass {
    pub op: PassOp,
    pub inputs: Box<[PlanResourceId]>,
    pub output: PlanResourceId,
    pub pipeline: PipelineKey,
    pub bindings: ResourceBindings,
    pub uniforms: PassUniforms,
    pub draw: DrawProgram,
}

impl Pass {
    fn draft(op: PassOp, inputs: Vec<PlanResourceId>, output: PlanResourceId) -> Self {
        Self {
            op,
            inputs: inputs.into_boxed_slice(),
            output,
            pipeline: PipelineKey::Filter,
            bindings: ResourceBindings::default(),
            uniforms: PassUniforms::default(),
            draw: DrawProgram {
                shader: DrawShader::Filter,
                viewport: PixelRect::default(),
                scissor: PixelRect::default(),
                vertex_count: 3,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlanResourceKind {
    External(ExternalResourceKind),
    Transient,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlanResource {
    pub kind: PlanResourceKind,
    pub coordinate_space: CoordinateSpace,
    pub logical_bounds: Rect,
    pub physical_bounds: PixelRect,
    pub texture_class: TextureClass,
    pub producer: Option<PassId>,
    pub last_read: Option<PassId>,
    pub slot: Option<TransientSlotId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransientSlot {
    pub texture_class: TextureClass,
    pub extent: Extent2d,
    pub resources: Box<[PlanResourceId]>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PlanStats {
    pub pass_count: usize,
    pub transient_resource_count: usize,
    pub transient_slot_count: usize,
    pub allocated_texels: u64,
    pub peak_live_texels: u64,
}

#[derive(Debug, Clone)]
pub struct LayerRenderPlan {
    passes: Box<[Pass]>,
    resources: Box<[PlanResource]>,
    slots: Box<[TransientSlot]>,
    backdrop: Option<PlanResourceId>,
    parent_destination: PlanResourceId,
    layer_content: PlanResourceId,
    stats: PlanStats,
}

impl LayerRenderPlan {
    pub fn passes(&self) -> &[Pass] {
        &self.passes
    }
    pub fn resources(&self) -> &[PlanResource] {
        &self.resources
    }
    pub fn slots(&self) -> &[TransientSlot] {
        &self.slots
    }
    pub const fn backdrop(&self) -> Option<PlanResourceId> {
        self.backdrop
    }
    pub const fn parent_destination(&self) -> PlanResourceId {
        self.parent_destination
    }
    pub const fn layer_content(&self) -> PlanResourceId {
        self.layer_content
    }
    pub const fn stats(&self) -> PlanStats {
        self.stats
    }
    pub fn is_noop(&self) -> bool {
        self.passes.is_empty()
    }
    pub fn external_resource(&self, kind: ExternalResourceKind) -> Option<PlanResourceId> {
        self.resources
            .iter()
            .position(|resource| resource.kind == PlanResourceKind::External(kind))
            .and_then(|index| u32::try_from(index).ok())
            .map(PlanResourceId)
    }
}

impl LayerProgram {
    /// Apply per-frame geometry, lower pass IR, crop demands, and allocate transients.
    pub fn instantiate(&self, context: &LayerPlanContext) -> Result<LayerRenderPlan, PlanError> {
        self.instantiate_entry(LayerProgramEntry::Complete, context)
    }

    /// Instantiate one explicit executable entry of this program.
    pub fn instantiate_entry(
        &self,
        entry: LayerProgramEntry,
        context: &LayerPlanContext,
    ) -> Result<LayerRenderPlan, PlanError> {
        validate_context(context)?;
        LoweredGraph::new(self, context)?
            .lower(self, entry, context)?
            .finish(context)
    }
}

#[derive(Clone)]
struct DraftResource {
    kind: PlanResourceKind,
    space: CoordinateSpace,
    full_bounds: Rect,
    class: TextureClass,
}

struct LoweredGraph {
    passes: Vec<Pass>,
    resources: Vec<DraftResource>,
    program_resources: Vec<Option<PlanResourceId>>,
    backdrop: Option<PlanResourceId>,
    parent_destination: PlanResourceId,
    layer_content: PlanResourceId,
}

impl LoweredGraph {
    fn new(program: &LayerProgram, context: &LayerPlanContext) -> Result<Self, PlanError> {
        let mut graph = Self {
            passes: Vec::with_capacity(program.nodes.len() * 3),
            resources: Vec::with_capacity(program.resources.len() * 2),
            program_resources: vec![None; program.resources.len()],
            backdrop: None,
            parent_destination: PlanResourceId(0),
            layer_content: PlanResourceId(0),
        };
        for (index, resource) in program.resources.iter().enumerate() {
            let ProgramResourceKind::External(kind) = resource.kind else {
                continue;
            };
            let (space, bounds, class) = match kind {
                ExternalResourceKind::Backdrop => (
                    CoordinateSpace::Parent,
                    context.backdrop_source_bounds,
                    context.color_texture_class,
                ),
                ExternalResourceKind::ParentDestination => (
                    CoordinateSpace::Parent,
                    context.parent_destination_bounds,
                    context.color_texture_class,
                ),
                ExternalResourceKind::LayerContent => (
                    CoordinateSpace::LayerLocal,
                    context.layer_content_bounds,
                    context.color_texture_class,
                ),
                ExternalResourceKind::BackdropMask => (
                    CoordinateSpace::Parent,
                    context
                        .backdrop_bounds
                        .unwrap_or(context.parent_destination_bounds),
                    TextureClass::MASK,
                ),
                ExternalResourceKind::LayerMask(_) => (
                    CoordinateSpace::LayerLocal,
                    expand_rect(context.layer_content_bounds, program.layer_expansion),
                    TextureClass::MASK,
                ),
            };
            let id = graph.add_resource(PlanResourceKind::External(kind), space, bounds, class)?;
            graph.program_resources[index] = Some(id);
        }
        graph.backdrop = program.backdrop.map(|id| graph.mapped(id)).transpose()?;
        graph.parent_destination = graph.mapped(program.parent_destination)?;
        graph.layer_content = graph.mapped(program.layer_content)?;
        Ok(graph)
    }

    fn lower(
        mut self,
        program: &LayerProgram,
        entry: LayerProgramEntry,
        context: &LayerPlanContext,
    ) -> Result<Self, PlanError> {
        let layer_start = program
            .nodes
            .iter()
            .rposition(|node| matches!(node.op, ProgramOp::BackdropComposite { .. }))
            .map_or(0, |index| index + 1);
        let layer_full_bounds = expand_rect(context.layer_content_bounds, program.layer_expansion);
        let composite_inverse = inverse_affine(context.composite.transform);
        let transformed_layer_bounds = composite_inverse.map(|_| {
            context
                .composite
                .transform
                .transform_rect(layer_full_bounds)
        });
        if transformed_layer_bounds.is_some_and(|bounds| !rect_is_finite(bounds)) {
            return Err(PlanError::CoordinateOverflow);
        }
        let layer_visible = transformed_layer_bounds.is_some_and(|bounds| {
            context.composite.opacity.clamp(0.0, 1.0) > 0.0
                && !is_empty(intersect_rect(
                    intersect_rect(bounds, context.parent_destination_bounds),
                    context
                        .composite_clip_bounds
                        .unwrap_or(context.parent_destination_bounds),
                ))
        });

        for (node_index, node) in program.nodes.iter().enumerate() {
            let in_backdrop_branch = node_index < layer_start;
            let selected = match entry {
                LayerProgramEntry::Complete => true,
                LayerProgramEntry::BackdropOnly => in_backdrop_branch,
                LayerProgramEntry::LayerOnly => !in_backdrop_branch,
            };
            if !selected {
                continue;
            }
            if node_index >= layer_start && !layer_visible {
                if matches!(node.op, ProgramOp::LayerComposite { .. }) {
                    self.map(node.output, self.parent_destination);
                } else {
                    self.map(node.output, self.mapped(node.inputs[0])?);
                }
                continue;
            }
            match &node.op {
                ProgramOp::Blur {
                    sigma_x,
                    sigma_y,
                    quality,
                } => {
                    let mut current = self.mapped(node.inputs[0])?;
                    if *sigma_x > 0.0 {
                        current = self.add_gaussian_passes(
                            current,
                            Axis::X,
                            scaled(*sigma_x, context.scale_factor)?,
                            quality.gaussian_support(),
                            expand_rect(
                                self.full_bounds(current),
                                SampleExpansion::symmetric(
                                    *sigma_x * quality.gaussian_support(),
                                    0.0,
                                ),
                            ),
                            self.space(current),
                            self.class(current),
                        )?;
                    }
                    if *sigma_y > 0.0 {
                        current = self.add_gaussian_passes(
                            current,
                            Axis::Y,
                            scaled(*sigma_y, context.scale_factor)?,
                            quality.gaussian_support(),
                            expand_rect(
                                self.full_bounds(current),
                                SampleExpansion::symmetric(
                                    0.0,
                                    *sigma_y * quality.gaussian_support(),
                                ),
                            ),
                            self.space(current),
                            self.class(current),
                        )?;
                    }
                    self.map(node.output, current);
                }
                ProgramOp::ColorMatrix(value) => {
                    self.lower_simple(node, PassOp::ColorMatrix(*value))?
                }
                ProgramOp::Pixelate { width, height } => self.lower_expanding(
                    node,
                    PassOp::Pixelate {
                        block_width_px: scaled(*width, context.scale_factor)?,
                        block_height_px: scaled(*height, context.scale_factor)?,
                    },
                    SampleExpansion::symmetric(*width * 0.5, *height * 0.5),
                )?,
                ProgramOp::Refraction {
                    strength,
                    chromatic_aberration,
                } => {
                    let amount = strength.abs() + chromatic_aberration.abs();
                    self.lower_expanding(
                        node,
                        PassOp::Refraction {
                            strength_px: scaled(*strength, context.scale_factor)?,
                            chromatic_aberration_px: scaled(
                                *chromatic_aberration,
                                context.scale_factor,
                            )?,
                        },
                        SampleExpansion::symmetric(amount, amount),
                    )?;
                }
                ProgramOp::ChromaticAberration { offset } => self.lower_expanding(
                    node,
                    PassOp::ChromaticAberration {
                        offset_px: [
                            scaled(offset[0], context.scale_factor)?,
                            scaled(offset[1], context.scale_factor)?,
                        ],
                    },
                    SampleExpansion::symmetric(offset[0].abs(), offset[1].abs()),
                )?,
                ProgramOp::DropShadow {
                    color,
                    offset,
                    sigma_x,
                    sigma_y,
                    spread,
                    quality,
                } => {
                    let original = self.mapped(node.inputs[0])?;
                    let mut alpha = self.add_filter_pass(
                        PassOp::ExtractAlpha,
                        original,
                        self.full_bounds(original),
                        self.space(original),
                        TextureClass::MASK,
                    )?;
                    if *spread > 0.0 {
                        alpha = self.add_spread_passes(
                            alpha,
                            scaled(*spread, context.scale_factor)?,
                            expand_rect(
                                self.full_bounds(alpha),
                                SampleExpansion::symmetric(*spread, *spread),
                            ),
                            self.space(alpha),
                            TextureClass::MASK,
                        )?;
                    }
                    if *sigma_x > 0.0 {
                        alpha = self.add_gaussian_passes(
                            alpha,
                            Axis::X,
                            scaled(*sigma_x, context.scale_factor)?,
                            quality.gaussian_support(),
                            expand_rect(
                                self.full_bounds(alpha),
                                SampleExpansion::symmetric(
                                    *sigma_x * quality.gaussian_support(),
                                    0.0,
                                ),
                            ),
                            self.space(alpha),
                            TextureClass::MASK,
                        )?;
                    }
                    if *sigma_y > 0.0 {
                        alpha = self.add_gaussian_passes(
                            alpha,
                            Axis::Y,
                            scaled(*sigma_y, context.scale_factor)?,
                            quality.gaussian_support(),
                            expand_rect(
                                self.full_bounds(alpha),
                                SampleExpansion::symmetric(
                                    0.0,
                                    *sigma_y * quality.gaussian_support(),
                                ),
                            ),
                            self.space(alpha),
                            TextureClass::MASK,
                        )?;
                    }
                    let shadow_bounds = translate_rect(self.full_bounds(alpha), offset.x, offset.y);
                    let output_bounds = self.full_bounds(original).union(shadow_bounds);
                    let output = self.add_transient(
                        CoordinateSpace::LayerLocal,
                        output_bounds,
                        context.color_texture_class,
                    )?;
                    self.passes.push(Pass::draft(
                        PassOp::ShadowComposite {
                            color: *color,
                            offset_px: [
                                scaled(offset.x, context.scale_factor)?,
                                scaled(offset.y, context.scale_factor)?,
                            ],
                        },
                        vec![original, alpha],
                        output,
                    ));
                    self.map(node.output, output);
                }
                ProgramOp::ApplyMask { transform, .. } => {
                    let input = self.mapped(node.inputs[0])?;
                    let mask = self.mapped(node.inputs[1])?;
                    let output = self.add_transient(
                        CoordinateSpace::LayerLocal,
                        self.full_bounds(input),
                        context.color_texture_class,
                    )?;
                    self.passes.push(Pass::draft(
                        PassOp::ApplyMask {
                            transform: *transform,
                            mask,
                        },
                        vec![input, mask],
                        output,
                    ));
                    self.map(node.output, output);
                }
                ProgramOp::BackdropComposite {
                    opacity,
                    blend_mode,
                    mask,
                } => {
                    let filtered = self.mapped(node.inputs[0])?;
                    let requested = context
                        .backdrop_bounds
                        .unwrap_or(context.parent_destination_bounds);
                    let bounds = intersect_rect(
                        intersect_rect(requested, context.parent_destination_bounds),
                        self.full_bounds(filtered),
                    );
                    if is_empty(bounds) {
                        self.map(node.output, self.parent_destination);
                        continue;
                    }
                    let mut inputs = vec![filtered];
                    if blend_mode.requires_destination_snapshot() {
                        inputs
                            .push(self.snapshot_destination(bounds, context.color_texture_class)?);
                    }
                    let plan_mask = self.lower_mask(mask)?;
                    if let PlanMask::Texture { resource, .. } = plan_mask {
                        inputs.push(resource);
                    }
                    self.passes.push(Pass::draft(
                        PassOp::BackdropComposite {
                            opacity: *opacity,
                            blend_mode: *blend_mode,
                            mask: plan_mask,
                            bounds,
                        },
                        inputs,
                        self.parent_destination,
                    ));
                    self.map(node.output, self.parent_destination);
                }
                ProgramOp::LayerComposite {
                    blend_mode,
                    operator,
                } => {
                    let layer = self.mapped(node.inputs[0])?;
                    let Some(inverse) = inverse_affine(context.composite.transform) else {
                        self.map(node.output, self.parent_destination);
                        continue;
                    };
                    let opacity = context.composite.opacity.clamp(0.0, 1.0);
                    if opacity == 0.0 {
                        self.map(node.output, self.parent_destination);
                        continue;
                    }
                    let transformed = context
                        .composite
                        .transform
                        .transform_rect(self.full_bounds(layer));
                    let bounds = intersect_rect(
                        intersect_rect(transformed, context.parent_destination_bounds),
                        context
                            .composite_clip_bounds
                            .unwrap_or(context.parent_destination_bounds),
                    );
                    if is_empty(bounds) {
                        self.map(node.output, self.parent_destination);
                        continue;
                    }
                    // Computing it here proves the reverse transform is finite for this demand.
                    let local_demand = inverse.transform_rect(bounds);
                    if !rect_is_finite(local_demand) {
                        return Err(PlanError::CoordinateOverflow);
                    }
                    let mut inputs = vec![layer];
                    if blend_mode.requires_destination_snapshot() {
                        inputs
                            .push(self.snapshot_destination(bounds, context.color_texture_class)?);
                    }
                    self.passes.push(Pass::draft(
                        PassOp::LayerComposite {
                            opacity,
                            transform: context.composite.transform,
                            blend_mode: *blend_mode,
                            operator: *operator,
                            bounds,
                        },
                        inputs,
                        self.parent_destination,
                    ));
                    self.map(node.output, self.parent_destination);
                }
            }
        }
        Ok(self)
    }

    fn lower_simple(&mut self, node: &crate::ProgramNode, op: PassOp) -> Result<(), PlanError> {
        let input = self.mapped(node.inputs[0])?;
        let output = self.add_filter_pass(
            op,
            input,
            self.full_bounds(input),
            self.space(input),
            self.class(input),
        )?;
        self.map(node.output, output);
        Ok(())
    }

    fn lower_expanding(
        &mut self,
        node: &crate::ProgramNode,
        op: PassOp,
        expansion: SampleExpansion,
    ) -> Result<(), PlanError> {
        let input = self.mapped(node.inputs[0])?;
        let output = self.add_filter_pass(
            op,
            input,
            expand_rect(self.full_bounds(input), expansion),
            self.space(input),
            self.class(input),
        )?;
        self.map(node.output, output);
        Ok(())
    }

    fn add_filter_pass(
        &mut self,
        op: PassOp,
        input: PlanResourceId,
        bounds: Rect,
        space: CoordinateSpace,
        class: TextureClass,
    ) -> Result<PlanResourceId, PlanError> {
        let output = self.add_transient(space, bounds, class)?;
        self.passes.push(Pass::draft(op, vec![input], output));
        Ok(output)
    }

    fn add_gaussian_passes(
        &mut self,
        mut input: PlanResourceId,
        axis: Axis,
        sigma_px: f32,
        support: f32,
        bounds: Rect,
        space: CoordinateSpace,
        class: TextureClass,
    ) -> Result<PlanResourceId, PlanError> {
        let stages = gaussian_stage_count(sigma_px, support);
        let stage_sigma = sigma_px / (stages as f32).sqrt();
        for _ in 0..stages {
            input = self.add_filter_pass(
                PassOp::GaussianBlur {
                    axis,
                    sigma_px: stage_sigma,
                    support,
                },
                input,
                bounds,
                space,
                class,
            )?;
        }
        Ok(input)
    }

    fn add_spread_passes(
        &mut self,
        mut input: PlanResourceId,
        radius_px: f32,
        bounds: Rect,
        space: CoordinateSpace,
        class: TextureClass,
    ) -> Result<PlanResourceId, PlanError> {
        let integer_radius = radius_px.ceil().max(0.0) as u32;
        let chunks = spread_chunk_count(integer_radius);
        for axis in [Axis::X, Axis::Y] {
            let mut remaining = integer_radius;
            for stage in 0..chunks {
                let stages_left = chunks - stage;
                let chunk = remaining.div_ceil(stages_left).min(128);
                remaining = remaining.saturating_sub(chunk);
                input = self.add_filter_pass(
                    PassOp::AlphaSpread {
                        axis,
                        radius_px: chunk as f32,
                    },
                    input,
                    bounds,
                    space,
                    class,
                )?;
            }
        }
        Ok(input)
    }

    fn snapshot_destination(
        &mut self,
        bounds: Rect,
        class: TextureClass,
    ) -> Result<PlanResourceId, PlanError> {
        let output = self.add_transient(CoordinateSpace::Parent, bounds, class)?;
        self.passes.push(Pass::draft(
            PassOp::Copy,
            vec![self.parent_destination],
            output,
        ));
        Ok(output)
    }

    fn lower_mask(&self, mask: &MaskProgram) -> Result<PlanMask, PlanError> {
        Ok(match mask {
            MaskProgram::None => PlanMask::None,
            MaskProgram::Shape { shape, transform } => PlanMask::Shape {
                shape: *shape,
                transform: *transform,
            },
            MaskProgram::Texture {
                transform,
                resource,
            } => PlanMask::Texture {
                transform: *transform,
                resource: self.mapped(*resource)?,
            },
        })
    }

    fn finish(self, context: &LayerPlanContext) -> Result<LayerRenderPlan, PlanError> {
        let mut demanded = vec![None; self.resources.len()];
        for pass in self.passes.iter().rev() {
            let output_demand = match pass.op {
                PassOp::BackdropComposite { bounds, .. }
                | PassOp::LayerComposite { bounds, .. } => bounds,
                _ => match demanded[pass.output.index()] {
                    Some(value) => value,
                    None => continue,
                },
            };
            let output_demand = intersect_rect(
                output_demand,
                self.resources[pass.output.index()].full_bounds,
            );
            propagate_inputs(
                pass,
                output_demand,
                context.scale_factor,
                &self.resources,
                &mut demanded,
            )?;
            union_into(&mut demanded[pass.output.index()], output_demand);
        }

        // Externals represent supplied texture domains, while transient bounds are demand-cropped.
        for (index, resource) in self.resources.iter().enumerate() {
            if matches!(resource.kind, PlanResourceKind::External(_)) && demanded[index].is_some() {
                demanded[index] = Some(intersect_rect(
                    demanded[index].expect("checked"),
                    resource.full_bounds,
                ));
            }
        }
        if self
            .passes
            .iter()
            .any(|pass| pass.output == self.parent_destination)
        {
            demanded[self.parent_destination.index()] = Some(context.parent_destination_bounds);
        }

        let mut resources = Vec::with_capacity(self.resources.len());
        for (index, draft) in self.resources.iter().enumerate() {
            let logical = demanded[index].unwrap_or(Rect::ZERO);
            let physical = physical_rect(logical, context.scale_factor)?;
            check_extent(physical.extent(), context.limits)?;
            resources.push(PlanResource {
                kind: draft.kind,
                coordinate_space: draft.space,
                logical_bounds: logical,
                physical_bounds: physical,
                texture_class: draft.class,
                producer: None,
                last_read: None,
                slot: None,
            });
        }
        for (index, pass) in self.passes.iter().enumerate() {
            let id = pass_id(index)?;
            resources[pass.output.index()].producer = Some(id);
            for input in pass.inputs.iter().copied() {
                let last = &mut resources[input.index()].last_read;
                if last.is_none_or(|previous| previous < id) {
                    *last = Some(id);
                }
            }
        }
        let mut passes = self.passes;
        for pass in &mut passes {
            configure_pass(pass, &resources, context.scale_factor)?;
        }
        let (slots, transient_count) = allocate_slots(&mut resources)?;
        let stats = PlanStats {
            pass_count: passes.len(),
            transient_resource_count: transient_count,
            transient_slot_count: slots.len(),
            allocated_texels: slots.iter().map(|slot| slot.extent.texels()).sum(),
            peak_live_texels: peak_live_texels(&resources, passes.len()),
        };
        Ok(LayerRenderPlan {
            passes: passes.into_boxed_slice(),
            resources: resources.into_boxed_slice(),
            slots: slots.into_boxed_slice(),
            backdrop: self.backdrop,
            parent_destination: self.parent_destination,
            layer_content: self.layer_content,
            stats,
        })
    }

    fn add_resource(
        &mut self,
        kind: PlanResourceKind,
        space: CoordinateSpace,
        bounds: Rect,
        class: TextureClass,
    ) -> Result<PlanResourceId, PlanError> {
        let id = plan_resource_id(self.resources.len())?;
        self.resources.push(DraftResource {
            kind,
            space,
            full_bounds: bounds,
            class,
        });
        Ok(id)
    }
    fn add_transient(
        &mut self,
        space: CoordinateSpace,
        bounds: Rect,
        class: TextureClass,
    ) -> Result<PlanResourceId, PlanError> {
        self.add_resource(PlanResourceKind::Transient, space, bounds, class)
    }
    fn mapped(&self, id: crate::ProgramResourceId) -> Result<PlanResourceId, PlanError> {
        self.program_resources[id.index()].ok_or(PlanError::InternalInvariant(
            "program resource was not lowered",
        ))
    }
    fn map(&mut self, id: crate::ProgramResourceId, value: PlanResourceId) {
        self.program_resources[id.index()] = Some(value);
    }
    fn full_bounds(&self, id: PlanResourceId) -> Rect {
        self.resources[id.index()].full_bounds
    }
    fn space(&self, id: PlanResourceId) -> CoordinateSpace {
        self.resources[id.index()].space
    }
    fn class(&self, id: PlanResourceId) -> TextureClass {
        self.resources[id.index()].class
    }
}

fn configure_pass(
    pass: &mut Pass,
    resources: &[PlanResource],
    scale_factor: f32,
) -> Result<(), PlanError> {
    pass.bindings = ResourceBindings {
        texture0: pass.inputs.first().copied(),
        texture1: pass.inputs.get(1).copied(),
        texture2: pass.inputs.get(2).copied(),
    };
    let output = resources[pass.output.index()].physical_bounds;
    pass.uniforms = PassUniforms {
        output_logical_bounds: resources[pass.output.index()].logical_bounds,
        output_physical_bounds: output,
        scale_factor,
    };
    let (pipeline, shader, viewport, scissor) = match &pass.op {
        PassOp::BackdropComposite {
            blend_mode,
            mask,
            bounds,
            ..
        } => {
            if let PlanMask::Texture { resource, .. } = mask {
                pass.bindings.texture2 = Some(*resource);
            }
            let (pipeline, shader) = if blend_mode.requires_destination_snapshot() {
                (AttachmentBlend::Replace, DrawShader::Composite)
            } else {
                (AttachmentBlend::SrcOver, DrawShader::AttachmentBackdrop)
            };
            (
                PipelineKey::Composite(pipeline),
                shader,
                local_extent(output),
                relative_physical_rect(*bounds, output, scale_factor)?,
            )
        }
        PassOp::LayerComposite {
            blend_mode,
            operator,
            bounds,
            ..
        } => {
            let (pipeline, shader) = if blend_mode.requires_destination_snapshot() {
                (AttachmentBlend::Replace, DrawShader::Composite)
            } else {
                (
                    match operator {
                        CompositeOperator::SrcOver => AttachmentBlend::SrcOver,
                        CompositeOperator::Src => AttachmentBlend::Replace,
                        CompositeOperator::DstOver => AttachmentBlend::DstOver,
                    },
                    DrawShader::AttachmentLayer,
                )
            };
            (
                PipelineKey::Composite(pipeline),
                shader,
                local_extent(output),
                relative_physical_rect(*bounds, output, scale_factor)?,
            )
        }
        PassOp::ApplyMask { mask, .. } => {
            pass.bindings.texture1 = Some(*mask);
            (
                PipelineKey::Filter,
                DrawShader::Filter,
                local_extent(output),
                local_extent(output),
            )
        }
        _ => (
            PipelineKey::Filter,
            DrawShader::Filter,
            local_extent(output),
            local_extent(output),
        ),
    };
    pass.pipeline = pipeline;
    pass.draw = DrawProgram {
        shader,
        viewport,
        scissor,
        vertex_count: 3,
    };
    Ok(())
}

fn local_extent(rect: PixelRect) -> PixelRect {
    PixelRect {
        x: 0,
        y: 0,
        width: rect.width,
        height: rect.height,
    }
}

fn relative_physical_rect(
    logical: Rect,
    target: PixelRect,
    scale_factor: f32,
) -> Result<PixelRect, PlanError> {
    let rect = physical_rect(logical, scale_factor)?;
    let left = i64::from(rect.x).max(i64::from(target.x));
    let top = i64::from(rect.y).max(i64::from(target.y));
    let right = (i64::from(rect.x) + i64::from(rect.width))
        .min(i64::from(target.x) + i64::from(target.width));
    let bottom = (i64::from(rect.y) + i64::from(rect.height))
        .min(i64::from(target.y) + i64::from(target.height));
    if right <= left || bottom <= top {
        return Ok(PixelRect::default());
    }
    Ok(PixelRect {
        x: i32::try_from(left - i64::from(target.x)).map_err(|_| PlanError::CoordinateOverflow)?,
        y: i32::try_from(top - i64::from(target.y)).map_err(|_| PlanError::CoordinateOverflow)?,
        width: u32::try_from(right - left).map_err(|_| PlanError::CoordinateOverflow)?,
        height: u32::try_from(bottom - top).map_err(|_| PlanError::CoordinateOverflow)?,
    })
}

fn propagate_inputs(
    pass: &Pass,
    demand: Rect,
    scale: f32,
    resources: &[DraftResource],
    demanded: &mut [Option<Rect>],
) -> Result<(), PlanError> {
    let mut needs = vec![demand; pass.inputs.len()];
    match pass.op {
        PassOp::GaussianBlur {
            axis,
            sigma_px,
            support,
        } => {
            let amount = sigma_px * support / scale;
            needs[0] = expand_rect(
                demand,
                match axis {
                    Axis::X => SampleExpansion::symmetric(amount, 0.0),
                    Axis::Y => SampleExpansion::symmetric(0.0, amount),
                },
            );
        }
        PassOp::Pixelate {
            block_width_px,
            block_height_px,
        } => {
            needs[0] = expand_rect(
                demand,
                SampleExpansion::symmetric(
                    block_width_px * 0.5 / scale,
                    block_height_px * 0.5 / scale,
                ),
            )
        }
        PassOp::Refraction {
            strength_px,
            chromatic_aberration_px,
        } => {
            let amount = (strength_px.abs() + chromatic_aberration_px.abs()) / scale;
            needs[0] = expand_rect(demand, SampleExpansion::symmetric(amount, amount));
        }
        PassOp::ChromaticAberration { offset_px } => {
            needs[0] = expand_rect(
                demand,
                SampleExpansion::symmetric(offset_px[0].abs() / scale, offset_px[1].abs() / scale),
            )
        }
        PassOp::AlphaSpread { radius_px, .. } => {
            let amount = radius_px / scale;
            needs[0] = expand_rect(demand, SampleExpansion::symmetric(amount, amount));
        }
        PassOp::ShadowComposite { offset_px, .. } => {
            needs[0] = demand;
            needs[1] = translate_rect(demand, -offset_px[0] / scale, -offset_px[1] / scale);
        }
        PassOp::LayerComposite { transform, .. } => {
            let inverse = inverse_affine(transform).ok_or(PlanError::InternalInvariant(
                "scheduled composite transform is singular",
            ))?;
            needs[0] = inverse.transform_rect(demand);
            if needs.len() > 1 {
                needs[1] = demand;
            }
        }
        PassOp::Copy
        | PassOp::ColorMatrix(_)
        | PassOp::ExtractAlpha
        | PassOp::ApplyMask { .. }
        | PassOp::BackdropComposite { .. } => {}
    }
    for (index, input) in pass.inputs.iter().copied().enumerate() {
        let need = intersect_rect(needs[index], resources[input.index()].full_bounds);
        union_into(&mut demanded[input.index()], need);
    }
    Ok(())
}

fn validate_context(context: &LayerPlanContext) -> Result<(), PlanError> {
    validate_rect(context.backdrop_source_bounds, "backdrop_source_bounds")?;
    validate_rect(
        context.parent_destination_bounds,
        "parent_destination_bounds",
    )?;
    if let Some(bounds) = context.composite_clip_bounds {
        validate_rect(bounds, "composite_clip_bounds")?;
    }
    validate_rect(context.layer_content_bounds, "layer_content_bounds")?;
    if let Some(bounds) = context.backdrop_bounds {
        validate_rect(bounds, "backdrop_bounds")?;
    }
    if !context.scale_factor.is_finite() || context.scale_factor <= 0.0 {
        return Err(PlanError::InvalidContext {
            field: "scale_factor",
            reason: "must be finite and greater than zero",
        });
    }
    if !context.composite.opacity.is_finite() {
        return Err(PlanError::InvalidContext {
            field: "composite.opacity",
            reason: "must be finite",
        });
    }
    let values = [
        context.composite.transform.xx,
        context.composite.transform.yx,
        context.composite.transform.xy,
        context.composite.transform.yy,
        context.composite.transform.dx,
        context.composite.transform.dy,
    ];
    if values.iter().any(|value| !value.is_finite()) {
        return Err(PlanError::InvalidContext {
            field: "composite.transform",
            reason: "must be finite",
        });
    }
    if context.limits.max_texture_dimension_2d == 0 {
        return Err(PlanError::InvalidContext {
            field: "limits.max_texture_dimension_2d",
            reason: "must be greater than zero",
        });
    }
    Ok(())
}

fn validate_rect(rect: Rect, field: &'static str) -> Result<(), PlanError> {
    if !rect_is_finite(rect) {
        return Err(PlanError::InvalidContext {
            field,
            reason: "must be finite",
        });
    }
    if rect.width < 0.0 || rect.height < 0.0 {
        return Err(PlanError::InvalidContext {
            field,
            reason: "width and height must not be negative",
        });
    }
    Ok(())
}
fn rect_is_finite(rect: Rect) -> bool {
    [rect.x, rect.y, rect.width, rect.height]
        .iter()
        .all(|value| value.is_finite())
}
fn is_empty(rect: Rect) -> bool {
    rect.width <= 0.0 || rect.height <= 0.0
}

fn inverse_affine(value: Affine) -> Option<Affine> {
    let determinant = value.xx * value.yy - value.xy * value.yx;
    if !determinant.is_finite() || determinant == 0.0 {
        return None;
    }
    let inverse = 1.0 / determinant;
    let xx = value.yy * inverse;
    let yx = -value.yx * inverse;
    let xy = -value.xy * inverse;
    let yy = value.xx * inverse;
    let dx = -(xx * value.dx + xy * value.dy);
    let dy = -(yx * value.dx + yy * value.dy);
    let result = Affine::new(xx, yx, xy, yy, dx, dy);
    [xx, yx, xy, yy, dx, dy]
        .iter()
        .all(|item| item.is_finite())
        .then_some(result)
}

fn scaled(value: f32, scale: f32) -> Result<f32, PlanError> {
    let result = value * scale;
    if result.is_finite() {
        Ok(if result == 0.0 { 0.0 } else { result })
    } else {
        Err(PlanError::CoordinateOverflow)
    }
}

fn gaussian_stage_count(sigma_px: f32, support: f32) -> u32 {
    (((sigma_px * support) / 128.0).max(1.0).powi(2).ceil() as u32).max(1)
}

fn spread_chunk_count(radius_px: u32) -> u32 {
    radius_px.div_ceil(128).max(1)
}

fn expand_rect(rect: Rect, value: SampleExpansion) -> Rect {
    Rect::new(
        rect.x - value.left,
        rect.y - value.top,
        rect.width + value.left + value.right,
        rect.height + value.top + value.bottom,
    )
}
fn translate_rect(rect: Rect, x: f32, y: f32) -> Rect {
    Rect::new(rect.x + x, rect.y + y, rect.width, rect.height)
}
fn intersect_rect(a: Rect, b: Rect) -> Rect {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.width).min(b.x + b.width);
    let y1 = (a.y + a.height).min(b.y + b.height);
    if x1 <= x0 || y1 <= y0 {
        Rect::new(x0, y0, 0.0, 0.0)
    } else {
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }
}
fn union_into(target: &mut Option<Rect>, value: Rect) {
    *target = Some(match *target {
        Some(existing) => existing.union(value),
        None => value,
    });
}

fn physical_rect(rect: Rect, scale: f32) -> Result<PixelRect, PlanError> {
    let x0 = (rect.x * scale).floor();
    let y0 = (rect.y * scale).floor();
    let x1 = ((rect.x + rect.width) * scale).ceil();
    let y1 = ((rect.y + rect.height) * scale).ceil();
    if [x0, y0, x1, y1].iter().any(|value| !value.is_finite())
        || x0 < i32::MIN as f32
        || y0 < i32::MIN as f32
        || x1 > i32::MAX as f32
        || y1 > i32::MAX as f32
        || x1 < x0
        || y1 < y0
    {
        return Err(PlanError::CoordinateOverflow);
    }
    Ok(PixelRect {
        x: x0 as i32,
        y: y0 as i32,
        width: (x1 as i64 - x0 as i64)
            .try_into()
            .map_err(|_| PlanError::CoordinateOverflow)?,
        height: (y1 as i64 - y0 as i64)
            .try_into()
            .map_err(|_| PlanError::CoordinateOverflow)?,
    })
}
fn check_extent(extent: Extent2d, limits: PlanLimits) -> Result<(), PlanError> {
    if extent.width > limits.max_texture_dimension_2d
        || extent.height > limits.max_texture_dimension_2d
    {
        Err(PlanError::TextureTooLarge {
            width: extent.width,
            height: extent.height,
            limit: limits.max_texture_dimension_2d,
        })
    } else {
        Ok(())
    }
}

struct SlotBuilder {
    class: TextureClass,
    extent: Extent2d,
    available_after: PassId,
    resources: Vec<PlanResourceId>,
}
fn allocate_slots(
    resources: &mut [PlanResource],
) -> Result<(Vec<TransientSlot>, usize), PlanError> {
    let mut transient: Vec<_> = resources
        .iter()
        .enumerate()
        .filter_map(|(index, resource)| {
            (resource.kind == PlanResourceKind::Transient).then_some((index, resource.producer))
        })
        .collect();
    transient.sort_by_key(|(index, producer)| (producer.map(|id| id.0), *index));
    let mut slots: Vec<SlotBuilder> = Vec::new();
    for (index, producer) in transient.iter().copied() {
        let producer = producer.ok_or(PlanError::InternalInvariant(
            "transient resource has no producer",
        ))?;
        let last_read = resources[index].last_read.unwrap_or(producer);
        let extent = resources[index].physical_bounds.extent();
        let class = resources[index].texture_class;
        let candidate = slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.class == class && slot.available_after < producer)
            .map(|(slot_index, slot)| {
                let grown = Extent2d {
                    width: slot.extent.width.max(extent.width),
                    height: slot.extent.height.max(extent.height),
                };
                (
                    slot_index,
                    grown.texels() - slot.extent.texels(),
                    grown.texels(),
                )
            })
            .min_by(|a, b| {
                a.1.cmp(&b.1)
                    .then_with(|| a.2.cmp(&b.2))
                    .then_with(|| a.0.cmp(&b.0))
            })
            .map(|value| value.0);
        let resource_id = plan_resource_id(index)?;
        let slot_index = if let Some(slot_index) = candidate {
            let slot = &mut slots[slot_index];
            slot.extent.width = slot.extent.width.max(extent.width);
            slot.extent.height = slot.extent.height.max(extent.height);
            slot.available_after = last_read;
            slot.resources.push(resource_id);
            slot_index
        } else {
            slots.push(SlotBuilder {
                class,
                extent,
                available_after: last_read,
                resources: vec![resource_id],
            });
            slots.len() - 1
        };
        resources[index].slot = Some(transient_slot_id(slot_index)?);
    }
    Ok((
        slots
            .into_iter()
            .map(|slot| TransientSlot {
                texture_class: slot.class,
                extent: slot.extent,
                resources: slot.resources.into_boxed_slice(),
            })
            .collect(),
        transient.len(),
    ))
}

fn peak_live_texels(resources: &[PlanResource], pass_count: usize) -> u64 {
    (0..pass_count)
        .map(|pass| {
            resources
                .iter()
                .filter(|resource| resource.kind == PlanResourceKind::Transient)
                .filter(|resource| {
                    let Some(first) = resource.producer else {
                        return false;
                    };
                    let last = resource.last_read.unwrap_or(first);
                    first.index() <= pass && pass <= last.index()
                })
                .map(|resource| resource.physical_bounds.extent().texels())
                .sum()
        })
        .max()
        .unwrap_or(0)
}

fn plan_resource_id(index: usize) -> Result<PlanResourceId, PlanError> {
    u32::try_from(index)
        .map(PlanResourceId)
        .map_err(|_| PlanError::TooManyItems)
}
fn pass_id(index: usize) -> Result<PassId, PlanError> {
    u32::try_from(index)
        .map(PassId)
        .map_err(|_| PlanError::TooManyItems)
}
fn transient_slot_id(index: usize) -> Result<TransientSlotId, PlanError> {
    u32::try_from(index)
        .map(TransientSlotId)
        .map_err(|_| PlanError::TooManyItems)
}
