use std::sync::Arc;

use super::{BackdropEffect, LayerCacheKey, LayerEffect};
use xui_interface::{Affine, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompositeStyle {
    pub opacity: f32,
    /// Transform applied when the completed layer texture is composited.
    /// It never changes the world transform of draws inside the layer.
    pub transform: Affine,
    pub blend_mode: BlendMode,
}

impl Default for CompositeStyle {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            transform: Affine::IDENTITY,
            blend_mode: BlendMode::Normal,
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

#[derive(Debug, Clone, PartialEq)]
pub struct LayerDescriptor {
    pub bounds: Option<Rect>,
    pub cache_key: Option<LayerCacheKey>,
    pub cache_policy: CachePolicy,
    pub composite: CompositeStyle,
    pub effects: Arc<[LayerEffect]>,
    pub backdrop_effects: Arc<[BackdropEffect]>,
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
            backdrop_effects: Arc::from([]),
            force_offscreen: false,
        }
    }
}

impl LayerDescriptor {
    /// Conservative first implementation. A later compiler may fold opacity
    /// into a single primitive when it can prove that doing so is equivalent.
    pub fn requires_isolation(&self) -> bool {
        self.force_offscreen
            || !self.effects.is_empty()
            || !self.backdrop_effects.is_empty()
            || self.cache_policy != CachePolicy::None
            || self.composite.blend_mode != BlendMode::Normal
            || self.composite.opacity != 1.0
            || self.composite.transform != Affine::IDENTITY
    }

    pub fn effect_expansion(&self) -> f32 {
        self.effects.iter().map(LayerEffect::visual_expansion).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequential_effects_accumulate_required_padding() {
        let descriptor = LayerDescriptor {
            effects: Arc::from([
                LayerEffect::Blur { sigma: 2.0 },
                LayerEffect::Blur { sigma: 3.0 },
            ]),
            ..LayerDescriptor::default()
        };
        assert_eq!(descriptor.effect_expansion(), 15.0);
    }
}
