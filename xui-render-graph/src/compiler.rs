use crate::{
    BackdropFilter, ColorMatrix, CompileError, ExternalResourceKind, LayerEffect,
    LayerGraphDescriptor, LayerProgram, Mask, MaskProgram, MaskShape, ProgramFingerprint,
    ProgramNode, ProgramNodeId, ProgramOp, ProgramResource, ProgramResourceId, ProgramResourceKind,
    SampleExpansion, WorkingColorSpace, matrix,
};
use std::f32::consts::{PI, TAU};
use xui_interface::{Affine, Color, Point};

const FINGERPRINT_VERSION: &[u8] = b"xui-render-graph/layer-program/v3";

/// Normalize and optimize a backend-independent layer descriptor.
pub fn compile_layer(descriptor: &LayerGraphDescriptor) -> Result<LayerProgram, CompileError> {
    validate_color_space(descriptor.working_color_space)?;
    let mut builder = ProgramBuilder::new()?;
    let mut backdrop_expansion = SampleExpansion::ZERO;

    if let Some(backdrop) = &descriptor.backdrop {
        let opacity = finite_style(backdrop.opacity, "backdrop.opacity")?.clamp(0.0, 1.0);
        if opacity > 0.0 {
            let mask = normalize_mask(&backdrop.mask)?;
            let mask_resource = if matches!(mask, NormalizedMask::Texture { .. }) {
                Some(builder.add_external(ExternalResourceKind::BackdropMask)?)
            } else {
                None
            };
            let mask = mask.into_program(mask_resource);
            let mut ops = Vec::with_capacity(backdrop.filters.len());
            for (index, filter) in backdrop.filters.iter().copied().enumerate() {
                if let Some(op) = normalize_backdrop_filter(index, filter)? {
                    ops.push((index, op));
                }
            }
            let backdrop_source = builder.add_backdrop()?;
            let (current, expansion) =
                build_effect_chain(&mut builder, backdrop_source, ops, ErrorBranch::Backdrop)?;
            backdrop_expansion = expansion;
            let mut inputs = vec![current, builder.parent_destination];
            if let Some(mask) = mask_resource {
                inputs.push(mask);
            }
            builder.add_virtual_node(
                ProgramOp::BackdropComposite {
                    opacity: matrix::canonical_zero(opacity),
                    blend_mode: backdrop.blend_mode,
                    mask,
                },
                &inputs,
            )?;
        }
    }

    let mut layer_ops = Vec::with_capacity(descriptor.effects.len());
    let mut mask_ordinal = 0_u32;
    for (index, effect) in descriptor.effects.iter().enumerate() {
        if let Some(op) = normalize_layer_effect(index, effect, mask_ordinal)? {
            if matches!(op, ProgramOp::ApplyMask { .. }) {
                builder.add_external(ExternalResourceKind::LayerMask(mask_ordinal))?;
                mask_ordinal = mask_ordinal
                    .checked_add(1)
                    .ok_or(CompileError::TooManyNodes)?;
            }
            layer_ops.push((index, op));
        }
    }
    let layer_content = builder.layer_content;
    let (layer_output, layer_expansion) =
        build_effect_chain(&mut builder, layer_content, layer_ops, ErrorBranch::Layer)?;
    builder.add_virtual_node(
        ProgramOp::LayerComposite {
            blend_mode: descriptor.composite.blend_mode,
            operator: descriptor.composite.operator,
        },
        &[layer_output, builder.parent_destination],
    )?;

    builder.finish(
        descriptor.working_color_space,
        backdrop_expansion,
        layer_expansion,
    )
}

#[derive(Clone, Copy)]
enum ErrorBranch {
    Backdrop,
    Layer,
}

fn build_effect_chain(
    builder: &mut ProgramBuilder,
    source: ProgramResourceId,
    ops: Vec<(usize, ProgramOp)>,
    branch: ErrorBranch,
) -> Result<(ProgramResourceId, SampleExpansion), CompileError> {
    let mut current = source;
    let mut pending_matrix = matrix::IDENTITY;
    let mut expansion = SampleExpansion::ZERO;
    for (index, op) in ops {
        if let ProgramOp::ColorMatrix(color_matrix) = op {
            let fused = matrix::compose(color_matrix, pending_matrix);
            if fused.iter().any(|value| !value.is_finite()) {
                return Err(operation_error(
                    branch,
                    index,
                    "matrix",
                    "color-matrix fusion overflowed",
                ));
            }
            pending_matrix = fused;
            continue;
        }
        flush_color_matrix(builder, &mut current, &mut pending_matrix)?;
        let next = expansion.then(op.sample_expansion());
        if !expansion_is_finite(next) {
            return Err(operation_error(
                branch,
                index,
                "sampling_expansion",
                "combined sampling expansion overflowed",
            ));
        }
        expansion = next;
        let mut inputs = vec![current];
        if let ProgramOp::ApplyMask { ordinal, .. } = op {
            let mask = builder
                .external(ExternalResourceKind::LayerMask(ordinal))
                .ok_or(CompileError::TooManyNodes)?;
            inputs.push(mask);
        }
        current = builder.add_virtual_node(op, &inputs)?;
    }
    flush_color_matrix(builder, &mut current, &mut pending_matrix)?;
    Ok((current, expansion))
}

fn flush_color_matrix(
    builder: &mut ProgramBuilder,
    current: &mut ProgramResourceId,
    pending: &mut ColorMatrix,
) -> Result<(), CompileError> {
    if !matrix::is_identity(pending) {
        *current = builder.add_virtual_node(ProgramOp::ColorMatrix(*pending), &[*current])?;
        *pending = matrix::IDENTITY;
    }
    Ok(())
}

fn normalize_backdrop_filter(
    index: usize,
    filter: BackdropFilter,
) -> Result<Option<ProgramOp>, CompileError> {
    let finite = |value: f32, field| finite_operation(value, ErrorBranch::Backdrop, index, field);
    let op = match filter {
        BackdropFilter::Blur {
            sigma_x,
            sigma_y,
            quality,
        } => {
            let sigma_x = finite(sigma_x, "sigma_x")?.max(0.0);
            let sigma_y = finite(sigma_y, "sigma_y")?.max(0.0);
            if sigma_x == 0.0 && sigma_y == 0.0 {
                return Ok(None);
            }
            ProgramOp::Blur {
                sigma_x,
                sigma_y,
                quality,
            }
        }
        BackdropFilter::Saturate(value) => color_op(
            matrix::saturate(finite(value, "amount")?.max(0.0)),
            1.0,
            value,
        )?,
        BackdropFilter::Brightness(value) => color_op(
            matrix::brightness(finite(value, "amount")?.max(0.0)),
            1.0,
            value,
        )?,
        BackdropFilter::Contrast(value) => color_op(
            matrix::contrast(finite(value, "amount")?.max(0.0)),
            1.0,
            value,
        )?,
        BackdropFilter::Grayscale(value) => {
            let amount = finite(value, "amount")?.clamp(0.0, 1.0);
            if amount == 0.0 {
                return Ok(None);
            }
            ProgramOp::ColorMatrix(matrix::grayscale(amount))
        }
        BackdropFilter::Sepia(value) => {
            let amount = finite(value, "amount")?.clamp(0.0, 1.0);
            if amount == 0.0 {
                return Ok(None);
            }
            ProgramOp::ColorMatrix(matrix::sepia(amount))
        }
        BackdropFilter::HueRotate(value) => {
            let amount = normalize_radians(finite(value, "radians")?);
            if amount == 0.0 {
                return Ok(None);
            }
            ProgramOp::ColorMatrix(matrix::hue_rotate(amount))
        }
        BackdropFilter::Invert(value) => {
            let amount = finite(value, "amount")?.clamp(0.0, 1.0);
            if amount == 0.0 {
                return Ok(None);
            }
            ProgramOp::ColorMatrix(matrix::invert(amount))
        }
        BackdropFilter::ColorMatrix(mut value) => {
            for item in &mut value {
                *item = finite(*item, "matrix")?;
            }
            value = matrix::canonicalize_matrix(value);
            if matrix::is_identity(&value) {
                return Ok(None);
            }
            ProgramOp::ColorMatrix(value)
        }
        BackdropFilter::Pixelate { size } => {
            let width = finite(size.width, "size.width")?;
            let height = finite(size.height, "size.height")?;
            if width <= 0.0 || height <= 0.0 {
                return Ok(None);
            }
            ProgramOp::Pixelate { width, height }
        }
        BackdropFilter::Refraction {
            strength,
            chromatic_aberration,
        } => {
            let strength = finite(strength, "strength")?;
            let chromatic_aberration = finite(chromatic_aberration, "chromatic_aberration")?.abs();
            if strength == 0.0 && chromatic_aberration == 0.0 {
                return Ok(None);
            }
            ProgramOp::Refraction {
                strength,
                chromatic_aberration,
            }
        }
        BackdropFilter::ChromaticAberration { mut offset } => {
            offset[0] = finite(offset[0], "offset.x")?;
            offset[1] = finite(offset[1], "offset.y")?;
            if offset == [0.0, 0.0] {
                return Ok(None);
            }
            ProgramOp::ChromaticAberration { offset }
        }
    };
    validate_expansion(ErrorBranch::Backdrop, index, &op)?;
    Ok(Some(op))
}

fn color_op(matrix_value: ColorMatrix, identity: f32, raw: f32) -> Result<ProgramOp, CompileError> {
    if raw == identity {
        Ok(ProgramOp::ColorMatrix(matrix::IDENTITY))
    } else {
        Ok(ProgramOp::ColorMatrix(matrix_value))
    }
}

fn normalize_layer_effect(
    index: usize,
    effect: &LayerEffect,
    mask_ordinal: u32,
) -> Result<Option<ProgramOp>, CompileError> {
    let finite = |value: f32, field| finite_operation(value, ErrorBranch::Layer, index, field);
    let op = match effect {
        LayerEffect::Blur {
            sigma_x,
            sigma_y,
            quality,
        } => {
            let sigma_x = finite(*sigma_x, "sigma_x")?.max(0.0);
            let sigma_y = finite(*sigma_y, "sigma_y")?.max(0.0);
            if sigma_x == 0.0 && sigma_y == 0.0 {
                return Ok(None);
            }
            ProgramOp::Blur {
                sigma_x,
                sigma_y,
                quality: *quality,
            }
        }
        LayerEffect::DropShadow {
            color,
            offset,
            sigma_x,
            sigma_y,
            spread,
            quality,
        } => {
            let color = normalize_color(*color, index)?;
            let offset = Point::new(finite(offset.x, "offset.x")?, finite(offset.y, "offset.y")?);
            let sigma_x = finite(*sigma_x, "sigma_x")?.max(0.0);
            let sigma_y = finite(*sigma_y, "sigma_y")?.max(0.0);
            let spread = finite(*spread, "spread")?.max(0.0);
            if color.a == 0.0 {
                return Ok(None);
            }
            ProgramOp::DropShadow {
                color,
                offset,
                sigma_x,
                sigma_y,
                spread,
                quality: *quality,
            }
        }
        LayerEffect::ColorMatrix(value) => {
            let mut value = *value;
            for item in &mut value {
                *item = finite(*item, "matrix")?;
            }
            value = matrix::canonicalize_matrix(value);
            if matrix::is_identity(&value) {
                return Ok(None);
            }
            ProgramOp::ColorMatrix(value)
        }
        LayerEffect::ImageMask { bounds, .. } => ProgramOp::ApplyMask {
            transform: normalize_transform(
                xui_interface::Affine::new(
                    bounds.width,
                    0.0,
                    0.0,
                    bounds.height,
                    bounds.x,
                    bounds.y,
                ),
                "layer_mask.transform",
            )?,
            ordinal: mask_ordinal,
        },
    };
    validate_expansion(ErrorBranch::Layer, index, &op)?;
    Ok(Some(op))
}

fn validate_expansion(
    branch: ErrorBranch,
    index: usize,
    op: &ProgramOp,
) -> Result<(), CompileError> {
    if expansion_is_finite(op.sample_expansion()) {
        Ok(())
    } else {
        Err(operation_error(
            branch,
            index,
            "sampling_expansion",
            "sampling expansion overflowed",
        ))
    }
}

fn normalize_color(color: Color, index: usize) -> Result<Color, CompileError> {
    Ok(Color::rgba(
        finite_operation(color.r, ErrorBranch::Layer, index, "color.r")?,
        finite_operation(color.g, ErrorBranch::Layer, index, "color.g")?,
        finite_operation(color.b, ErrorBranch::Layer, index, "color.b")?,
        finite_operation(color.a, ErrorBranch::Layer, index, "color.a")?.clamp(0.0, 1.0),
    ))
}

#[derive(Debug, Clone)]
enum NormalizedMask {
    None,
    Shape { shape: MaskShape, transform: Affine },
    Texture { transform: Affine },
}
impl NormalizedMask {
    fn into_program(self, resource: Option<ProgramResourceId>) -> MaskProgram {
        match self {
            Self::None => MaskProgram::None,
            Self::Shape { shape, transform } => MaskProgram::Shape { shape, transform },
            Self::Texture { transform } => MaskProgram::Texture {
                transform,
                resource: resource.expect("mask resource exists"),
            },
        }
    }
}

fn normalize_mask(mask: &Mask) -> Result<NormalizedMask, CompileError> {
    match mask {
        Mask::None => Ok(NormalizedMask::None),
        Mask::Shape { shape, transform } => Ok(NormalizedMask::Shape {
            shape: normalize_shape(*shape)?,
            transform: normalize_transform(*transform, "backdrop.mask.transform")?,
        }),
        Mask::AlphaTexture { transform, .. } => Ok(NormalizedMask::Texture {
            transform: normalize_transform(*transform, "backdrop.mask.transform")?,
        }),
    }
}

fn normalize_shape(shape: MaskShape) -> Result<MaskShape, CompileError> {
    Ok(match shape {
        MaskShape::Rect => MaskShape::Rect,
        MaskShape::RoundedRect(radius) => {
            MaskShape::RoundedRect(finite_style(radius, "backdrop.mask.radius")?.max(0.0))
        }
        MaskShape::Circle => MaskShape::Circle,
        MaskShape::Ellipse => MaskShape::Ellipse,
        MaskShape::Line { from, to } => MaskShape::Line {
            from: Point::new(
                finite_style(from.x, "backdrop.mask.line.from.x")?,
                finite_style(from.y, "backdrop.mask.line.from.y")?,
            ),
            to: Point::new(
                finite_style(to.x, "backdrop.mask.line.to.x")?,
                finite_style(to.y, "backdrop.mask.line.to.y")?,
            ),
        },
    })
}

fn normalize_transform(transform: Affine, field: &'static str) -> Result<Affine, CompileError> {
    let values = [
        transform.xx,
        transform.yx,
        transform.xy,
        transform.yy,
        transform.dx,
        transform.dy,
    ];
    if values.iter().any(|value| !value.is_finite()) {
        return Err(CompileError::InvalidStyleParameter {
            field,
            reason: "must be finite",
        });
    }
    let determinant = transform.xx * transform.yy - transform.xy * transform.yx;
    if determinant == 0.0 || !determinant.is_finite() {
        return Err(CompileError::InvalidStyleParameter {
            field,
            reason: "must be invertible",
        });
    }
    Ok(Affine::new(
        matrix::canonical_zero(transform.xx),
        matrix::canonical_zero(transform.yx),
        matrix::canonical_zero(transform.xy),
        matrix::canonical_zero(transform.yy),
        matrix::canonical_zero(transform.dx),
        matrix::canonical_zero(transform.dy),
    ))
}

fn finite_style(value: f32, field: &'static str) -> Result<f32, CompileError> {
    if value.is_finite() {
        Ok(matrix::canonical_zero(value))
    } else {
        Err(CompileError::InvalidStyleParameter {
            field,
            reason: "must be finite",
        })
    }
}

fn finite_operation(
    value: f32,
    branch: ErrorBranch,
    index: usize,
    field: &'static str,
) -> Result<f32, CompileError> {
    if value.is_finite() {
        Ok(matrix::canonical_zero(value))
    } else {
        Err(operation_error(branch, index, field, "must be finite"))
    }
}

fn operation_error(
    branch: ErrorBranch,
    index: usize,
    field: &'static str,
    reason: &'static str,
) -> CompileError {
    match branch {
        ErrorBranch::Backdrop => CompileError::InvalidFilterParameter {
            index,
            field,
            reason,
        },
        ErrorBranch::Layer => CompileError::InvalidEffectParameter {
            index,
            field,
            reason,
        },
    }
}

fn validate_color_space(value: WorkingColorSpace) -> Result<(), CompileError> {
    match value {
        WorkingColorSpace::LinearScene => Ok(()),
    }
}

fn normalize_radians(value: f32) -> f32 {
    let normalized = (value + PI).rem_euclid(TAU) - PI;
    if normalized.abs() <= 1.0e-6 {
        0.0
    } else {
        matrix::canonical_zero(normalized)
    }
}

fn expansion_is_finite(value: SampleExpansion) -> bool {
    [value.left, value.top, value.right, value.bottom]
        .iter()
        .all(|item| item.is_finite())
}

struct ProgramBuilder {
    nodes: Vec<ProgramNode>,
    resources: Vec<ProgramResource>,
    backdrop: Option<ProgramResourceId>,
    parent_destination: ProgramResourceId,
    layer_content: ProgramResourceId,
}

impl ProgramBuilder {
    fn new() -> Result<Self, CompileError> {
        let mut value = Self {
            nodes: Vec::new(),
            resources: Vec::new(),
            backdrop: None,
            parent_destination: ProgramResourceId(0),
            layer_content: ProgramResourceId(0),
        };
        value.parent_destination = value.add_external(ExternalResourceKind::ParentDestination)?;
        value.layer_content = value.add_external(ExternalResourceKind::LayerContent)?;
        Ok(value)
    }

    fn add_backdrop(&mut self) -> Result<ProgramResourceId, CompileError> {
        if let Some(id) = self.backdrop {
            return Ok(id);
        }
        let id = self.add_external(ExternalResourceKind::Backdrop)?;
        self.backdrop = Some(id);
        Ok(id)
    }

    fn add_external(
        &mut self,
        kind: ExternalResourceKind,
    ) -> Result<ProgramResourceId, CompileError> {
        let id = resource_id(self.resources.len())?;
        self.resources.push(ProgramResource {
            kind: ProgramResourceKind::External(kind),
            producer: None,
        });
        Ok(id)
    }

    fn external(&self, kind: ExternalResourceKind) -> Option<ProgramResourceId> {
        self.resources
            .iter()
            .position(|resource| resource.kind == ProgramResourceKind::External(kind))
            .and_then(|index| u32::try_from(index).ok())
            .map(ProgramResourceId)
    }

    fn add_virtual_node(
        &mut self,
        op: ProgramOp,
        inputs: &[ProgramResourceId],
    ) -> Result<ProgramResourceId, CompileError> {
        let node = node_id(self.nodes.len())?;
        let output = resource_id(self.resources.len())?;
        self.resources.push(ProgramResource {
            kind: ProgramResourceKind::Virtual,
            producer: Some(node),
        });
        self.nodes.push(ProgramNode {
            op,
            inputs: inputs.into(),
            output,
        });
        Ok(output)
    }

    fn finish(
        self,
        working_color_space: WorkingColorSpace,
        backdrop_expansion: SampleExpansion,
        layer_expansion: SampleExpansion,
    ) -> Result<LayerProgram, CompileError> {
        let fingerprint = fingerprint(
            &self.nodes,
            &self.resources,
            working_color_space,
            backdrop_expansion,
            layer_expansion,
        );
        Ok(LayerProgram {
            nodes: self.nodes.into_boxed_slice(),
            resources: self.resources.into_boxed_slice(),
            backdrop: self.backdrop,
            parent_destination: self.parent_destination,
            layer_content: self.layer_content,
            fingerprint,
            working_color_space,
            backdrop_expansion,
            layer_expansion,
        })
    }
}

fn node_id(index: usize) -> Result<ProgramNodeId, CompileError> {
    u32::try_from(index)
        .map(ProgramNodeId)
        .map_err(|_| CompileError::TooManyNodes)
}
fn resource_id(index: usize) -> Result<ProgramResourceId, CompileError> {
    u32::try_from(index)
        .map(ProgramResourceId)
        .map_err(|_| CompileError::TooManyNodes)
}

fn fingerprint(
    nodes: &[ProgramNode],
    resources: &[ProgramResource],
    color_space: WorkingColorSpace,
    backdrop: SampleExpansion,
    layer: SampleExpansion,
) -> ProgramFingerprint {
    let mut hash = blake3::Hasher::new();
    hash.update(FINGERPRINT_VERSION);
    hash.update(&[match color_space {
        WorkingColorSpace::LinearScene => 0,
    }]);
    hash.update(&(resources.len() as u64).to_le_bytes());
    for resource in resources {
        match resource.kind {
            ProgramResourceKind::Virtual => {
                hash.update(&[0]);
            }
            ProgramResourceKind::External(kind) => {
                hash.update(&[1]);
                hash_external(&mut hash, kind);
            }
        }
        hash.update(
            &resource
                .producer
                .map(|id| id.0)
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
    }
    hash.update(&(nodes.len() as u64).to_le_bytes());
    for node in nodes {
        hash_op(&mut hash, &node.op);
        hash.update(&(node.inputs.len() as u64).to_le_bytes());
        for input in &node.inputs {
            hash.update(&input.0.to_le_bytes());
        }
        hash.update(&node.output.0.to_le_bytes());
    }
    for value in [
        backdrop.left,
        backdrop.top,
        backdrop.right,
        backdrop.bottom,
        layer.left,
        layer.top,
        layer.right,
        layer.bottom,
    ] {
        hash_f32(&mut hash, value);
    }
    ProgramFingerprint(*hash.finalize().as_bytes())
}

fn hash_external(hash: &mut blake3::Hasher, kind: ExternalResourceKind) {
    match kind {
        ExternalResourceKind::Backdrop => {
            hash.update(&[0]);
        }
        ExternalResourceKind::ParentDestination => {
            hash.update(&[1]);
        }
        ExternalResourceKind::LayerContent => {
            hash.update(&[2]);
        }
        ExternalResourceKind::BackdropMask => {
            hash.update(&[3]);
        }
        ExternalResourceKind::LayerMask(value) => {
            hash.update(&[4]);
            hash.update(&value.to_le_bytes());
        }
    }
}

fn hash_op(hash: &mut blake3::Hasher, op: &ProgramOp) {
    match op {
        ProgramOp::Blur {
            sigma_x,
            sigma_y,
            quality,
        } => {
            hash.update(&[0, *quality as u8]);
            hash_f32(hash, *sigma_x);
            hash_f32(hash, *sigma_y);
        }
        ProgramOp::ColorMatrix(value) => {
            hash.update(&[1]);
            for item in value {
                hash_f32(hash, *item);
            }
        }
        ProgramOp::Pixelate { width, height } => {
            hash.update(&[2]);
            hash_f32(hash, *width);
            hash_f32(hash, *height);
        }
        ProgramOp::Refraction {
            strength,
            chromatic_aberration,
        } => {
            hash.update(&[3]);
            hash_f32(hash, *strength);
            hash_f32(hash, *chromatic_aberration);
        }
        ProgramOp::ChromaticAberration { offset } => {
            hash.update(&[4]);
            hash_f32(hash, offset[0]);
            hash_f32(hash, offset[1]);
        }
        ProgramOp::DropShadow {
            color,
            offset,
            sigma_x,
            sigma_y,
            spread,
            quality,
        } => {
            hash.update(&[5, *quality as u8]);
            for value in [
                color.r, color.g, color.b, color.a, offset.x, offset.y, *sigma_x, *sigma_y, *spread,
            ] {
                hash_f32(hash, value);
            }
        }
        ProgramOp::ApplyMask { transform, ordinal } => {
            hash.update(&[6]);
            hash_affine(hash, *transform);
            hash.update(&ordinal.to_le_bytes());
        }
        ProgramOp::BackdropComposite {
            opacity,
            blend_mode,
            mask,
        } => {
            hash.update(&[7, *blend_mode as u8]);
            hash_f32(hash, *opacity);
            hash_mask(hash, mask);
        }
        ProgramOp::LayerComposite {
            blend_mode,
            operator,
        } => {
            hash.update(&[8, *blend_mode as u8, *operator as u8]);
        }
    }
}

fn hash_mask(hash: &mut blake3::Hasher, mask: &MaskProgram) {
    match mask {
        MaskProgram::None => {
            hash.update(&[0]);
        }
        MaskProgram::Shape { shape, transform } => {
            hash.update(&[1]);
            hash_shape(hash, *shape);
            hash_affine(hash, *transform);
        }
        MaskProgram::Texture {
            transform,
            resource,
        } => {
            hash.update(&[2]);
            hash_affine(hash, *transform);
            hash.update(&resource.0.to_le_bytes());
        }
    }
}

fn hash_shape(hash: &mut blake3::Hasher, shape: MaskShape) {
    match shape {
        MaskShape::Rect => {
            hash.update(&[0]);
        }
        MaskShape::RoundedRect(value) => {
            hash.update(&[1]);
            hash_f32(hash, value);
        }
        MaskShape::Circle => {
            hash.update(&[2]);
        }
        MaskShape::Ellipse => {
            hash.update(&[3]);
        }
        MaskShape::Line { from, to } => {
            hash.update(&[4]);
            for value in [from.x, from.y, to.x, to.y] {
                hash_f32(hash, value);
            }
        }
    }
}

fn hash_affine(hash: &mut blake3::Hasher, value: Affine) {
    for item in [value.xx, value.yx, value.xy, value.yy, value.dx, value.dy] {
        hash_f32(hash, item);
    }
}
fn hash_f32(hash: &mut blake3::Hasher, value: f32) {
    hash.update(&matrix::canonical_zero(value).to_bits().to_le_bytes());
}
