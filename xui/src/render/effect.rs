use xui_interface::{ComputedShadowStyle, ImageData, ImageKey, Rect};

#[derive(Debug, Clone, PartialEq)]
pub enum LayerEffect {
    Blur {
        sigma: f32,
    },
    DropShadow(ComputedShadowStyle),
    ColorMatrix {
        matrix: [f32; 20],
    },
    Mask {
        image: ImageKey,
        data: ImageData,
        bounds: Rect,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum BackdropEffect {
    Blur { sigma: f32 },
}

impl LayerEffect {
    pub fn visual_expansion(&self) -> f32 {
        match self {
            Self::Blur { sigma } => sigma.max(0.0) * 3.0,
            Self::DropShadow(shadow) => {
                shadow.offset.x.abs().max(shadow.offset.y.abs())
                    + shadow.blur.max(0.0) * 3.0
                    + shadow.spread.max(0.0)
            }
            Self::ColorMatrix { .. } | Self::Mask { .. } => 0.0,
        }
    }
}

impl BackdropEffect {
    pub fn sampling_expansion(&self) -> f32 {
        match self {
            Self::Blur { sigma } => sigma.max(0.0) * 3.0,
        }
    }
}
