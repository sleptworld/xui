use xui_interface::{PathData, Rect};

#[derive(Debug, Clone, PartialEq)]
pub enum ClipShape {
    Rect(Rect),
    RoundedRect { rect: Rect, radius: f32 },
    Path { path: PathData, bounds: Rect },
}

impl ClipShape {
    pub fn local_bounds(&self) -> Rect {
        match self {
            Self::Rect(rect) | Self::RoundedRect { rect, .. } => *rect,
            Self::Path { bounds, .. } => *bounds,
        }
    }
}
