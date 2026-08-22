use super::{BlendMode, LayerCacheKey};
use crate::render::CompositeOperator;
use std::sync::Arc;
use xui_interface::{Affine, Bounds, ComputedBackdropStyle, ComputedEffect, Rect};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompositeStyle {
    pub opacity: f32,
    /// Transform applied when the completed layer texture is composited.
    /// It never changes the world transform of draws inside the layer.
    pub transform: Affine,
    pub blend_mode: BlendMode,
    pub operator: CompositeOperator,
}

impl Default for CompositeStyle {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            transform: Affine::IDENTITY,
            blend_mode: BlendMode::default(),
            operator: CompositeOperator::SrcOver,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CachePolicy {
    #[default]
    None,
    Auto,
    Always,
}

/// Controls whether descendants may observe destination history outside this layer.
///
/// This does not affect the backdrop input of the layer itself. An isolated layer may
/// still sample the destination that existed before it was composited into its parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BackdropIsolation {
    #[default]
    Passthrough,
    Isolate,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayerDescriptor {
    pub bounds: Option<Bounds>,
    pub cache_key: Option<LayerCacheKey>,
    pub cache_policy: CachePolicy,
    pub composite: CompositeStyle,
    pub backdrop_style: Option<ComputedBackdropStyle>,
    pub backdrop_isolation: BackdropIsolation,
    pub effects: Arc<[ComputedEffect]>,
    pub force_offscreen: bool,
}

impl Default for LayerDescriptor {
    fn default() -> Self {
        Self {
            bounds: None,
            cache_key: None,
            cache_policy: CachePolicy::None,
            composite: CompositeStyle::default(),
            effects: Arc::from([]),
            backdrop_style: None,
            backdrop_isolation: BackdropIsolation::Passthrough,
            // backdrop_effects: Arc::from([]),
            force_offscreen: false,
        }
    }
}

impl LayerDescriptor {
    /// Conservative first implementation. A later compiler may fold opacity
    /// into a single primitive when it can prove that doing so is equivalent.
    pub fn requires_isolation(&self) -> bool {
        self.force_offscreen
            || self.backdrop_isolation == BackdropIsolation::Isolate
            || !self.effects.is_empty()
            || self.backdrop_style.is_some()
            || self.cache_policy != CachePolicy::None
            || self.composite.blend_mode != BlendMode::Normal
            || self.composite.operator != CompositeOperator::SrcOver
            || self.composite.opacity != 1.0
            || self.composite.transform != Affine::IDENTITY
    }
}
