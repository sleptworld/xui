pub use xui_render_graph::{
    BackdropDescriptor, BackdropFilter, BlendMode, CompositeOperator, FilterQuality, LayerEffect,
    Mask,
};

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct SampleExpansion {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl SampleExpansion {
    pub const ZERO: Self = Self {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };

    pub fn symmetric(x: f32, y: f32) -> Self {
        let x = x.max(0.0);
        let y = y.max(0.0);

        Self {
            left: x,
            right: x,
            top: y,
            bottom: y,
        }
    }

    pub fn then(self, next: Self) -> Self {
        Self {
            left: self.left + next.left,
            top: self.top + next.top,
            right: self.right + next.right,
            bottom: self.bottom + next.bottom,
        }
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }
}
