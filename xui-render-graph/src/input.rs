use xui_interface::Affine;
pub use xui_interface::{
    BlendMode, ColorMatrix, ComputedBackdropFilter as BackdropFilter, ComputedBackdropMask as Mask,
    ComputedEffect as LayerEffect, ComputedMaskShape as MaskShape, FilterQuality,
};

/// Complete static description of one isolated layer.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerGraphDescriptor {
    pub backdrop: Option<BackdropDescriptor>,
    pub effects: Vec<LayerEffect>,
    pub composite: CompositeDescriptor,
    pub working_color_space: WorkingColorSpace,
}

impl Default for LayerGraphDescriptor {
    fn default() -> Self {
        Self {
            backdrop: None,
            effects: Vec::new(),
            composite: CompositeDescriptor::default(),
            working_color_space: WorkingColorSpace::LinearScene,
        }
    }
}

/// Backdrop branch evaluated before layer content is composited.
#[derive(Debug, Clone, PartialEq)]
pub struct BackdropDescriptor {
    pub filters: Vec<BackdropFilter>,
    pub opacity: f32,
    pub blend_mode: BlendMode,
    pub mask: Mask,
}

impl Default for BackdropDescriptor {
    fn default() -> Self {
        Self {
            filters: Vec::new(),
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: Mask::None,
        }
    }
}

/// Static part of the final layer composite.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CompositeDescriptor {
    pub blend_mode: BlendMode,
    pub operator: CompositeOperator,
}

/// Per-frame part of the final layer composite. It is excluded from fingerprints.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompositeInstance {
    pub opacity: f32,
    pub transform: Affine,
}

impl Default for CompositeInstance {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            transform: Affine::IDENTITY,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkingColorSpace {
    LinearScene,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum CompositeOperator {
    #[default]
    SrcOver,
    Src,
    DstOver,
}
