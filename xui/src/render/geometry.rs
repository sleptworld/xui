use xui_interface::{Bounds, PathData};

#[derive(Debug, Clone, PartialEq)]
pub enum ClipShape {
    Rect(Bounds),
    RoundedRect { rect: Bounds, radius: f32 },
    Path { path: PathData, bounds: Bounds },
}

impl ClipShape {
    pub fn local_bounds(&self) -> Bounds {
        match self {
            Self::Rect(rect) | Self::RoundedRect { rect, .. } => *rect,
            Self::Path { bounds, .. } => *bounds,
        }
    }
}
