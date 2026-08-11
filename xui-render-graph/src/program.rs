use crate::{
    BlendMode, ColorMatrix, CompositeOperator, FilterQuality, MaskShape, WorkingColorSpace,
};
use xui_interface::{Affine, Color, Point};

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

    pub const fn symmetric(x: f32, y: f32) -> Self {
        Self {
            left: x,
            top: y,
            right: x,
            bottom: y,
        }
    }

    pub const fn then(self, next: Self) -> Self {
        Self {
            left: self.left + next.left,
            top: self.top + next.top,
            right: self.right + next.right,
            bottom: self.bottom + next.bottom,
        }
    }

    pub const fn union(self, other: Self) -> Self {
        Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    pub fn to_bits(&self) -> [u32; 4] {
        [
            self.left.to_bits(),
            self.right.to_bits(),
            self.top.to_bits(),
            self.bottom.to_bits(),
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProgramFingerprint(pub(crate) [u8; 32]);

impl ProgramFingerprint {
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProgramNodeId(pub(crate) u32);
impl ProgramNodeId {
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProgramResourceId(pub(crate) u32);
impl ProgramResourceId {
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExternalResourceKind {
    Backdrop,
    ParentDestination,
    LayerContent,
    BackdropMask,
    LayerMask(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProgramResourceKind {
    External(ExternalResourceKind),
    Virtual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramResource {
    pub kind: ProgramResourceKind,
    pub producer: Option<ProgramNodeId>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MaskProgram {
    None,
    Shape {
        shape: MaskShape,
        transform: Affine,
    },
    Texture {
        transform: Affine,
        resource: ProgramResourceId,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProgramOp {
    Blur {
        sigma_x: f32,
        sigma_y: f32,
        quality: FilterQuality,
    },
    ColorMatrix(ColorMatrix),
    Pixelate {
        width: f32,
        height: f32,
    },
    Refraction {
        strength: f32,
        chromatic_aberration: f32,
    },
    ChromaticAberration {
        offset: [f32; 2],
    },
    DropShadow {
        color: Color,
        offset: Point,
        sigma_x: f32,
        sigma_y: f32,
        spread: f32,
        quality: FilterQuality,
    },
    ApplyMask {
        transform: Affine,
        ordinal: u32,
    },
    BackdropComposite {
        opacity: f32,
        blend_mode: BlendMode,
        mask: MaskProgram,
    },
    LayerComposite {
        blend_mode: BlendMode,
        operator: CompositeOperator,
    },
}

impl ProgramOp {
    pub fn sample_expansion(&self) -> SampleExpansion {
        match *self {
            Self::Blur {
                sigma_x,
                sigma_y,
                quality,
            } => SampleExpansion::symmetric(
                sigma_x * quality.gaussian_support(),
                sigma_y * quality.gaussian_support(),
            ),
            Self::Pixelate { width, height } => {
                SampleExpansion::symmetric(width * 0.5, height * 0.5)
            }
            Self::Refraction {
                strength,
                chromatic_aberration,
            } => {
                let amount = strength.abs() + chromatic_aberration.abs();
                SampleExpansion::symmetric(amount, amount)
            }
            Self::ChromaticAberration { offset } => {
                SampleExpansion::symmetric(offset[0].abs(), offset[1].abs())
            }
            Self::DropShadow {
                offset,
                sigma_x,
                sigma_y,
                spread,
                quality,
                ..
            } => {
                let x = spread + sigma_x * quality.gaussian_support();
                let y = spread + sigma_y * quality.gaussian_support();
                SampleExpansion {
                    left: (x - offset.x).max(0.0),
                    top: (y - offset.y).max(0.0),
                    right: (x + offset.x).max(0.0),
                    bottom: (y + offset.y).max(0.0),
                }
            }
            Self::ColorMatrix(_)
            | Self::ApplyMask { .. }
            | Self::BackdropComposite { .. }
            | Self::LayerComposite { .. } => SampleExpansion::ZERO,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProgramNode {
    pub op: ProgramOp,
    pub inputs: Box<[ProgramResourceId]>,
    pub output: ProgramResourceId,
}

/// Immutable normalized static program shared by many frames.
#[derive(Debug, Clone)]
pub struct LayerProgram {
    pub(crate) nodes: Box<[ProgramNode]>,
    pub(crate) resources: Box<[ProgramResource]>,
    pub(crate) backdrop: Option<ProgramResourceId>,
    pub(crate) parent_destination: ProgramResourceId,
    pub(crate) layer_content: ProgramResourceId,
    pub(crate) fingerprint: ProgramFingerprint,
    pub(crate) working_color_space: WorkingColorSpace,
    pub(crate) backdrop_expansion: SampleExpansion,
    pub(crate) layer_expansion: SampleExpansion,
}

impl LayerProgram {
    pub fn nodes(&self) -> &[ProgramNode] {
        &self.nodes
    }
    pub fn resources(&self) -> &[ProgramResource] {
        &self.resources
    }
    pub const fn backdrop(&self) -> Option<ProgramResourceId> {
        self.backdrop
    }
    pub const fn parent_destination(&self) -> ProgramResourceId {
        self.parent_destination
    }
    pub const fn layer_content(&self) -> ProgramResourceId {
        self.layer_content
    }
    pub const fn fingerprint(&self) -> ProgramFingerprint {
        self.fingerprint
    }
    pub const fn working_color_space(&self) -> WorkingColorSpace {
        self.working_color_space
    }
    pub const fn backdrop_input_expansion(&self) -> SampleExpansion {
        self.backdrop_expansion
    }
    pub const fn layer_visual_expansion(&self) -> SampleExpansion {
        self.layer_expansion
    }
    pub fn external_resource(&self, kind: ExternalResourceKind) -> Option<ProgramResourceId> {
        self.resources
            .iter()
            .position(|resource| resource.kind == ProgramResourceKind::External(kind))
            .and_then(|index| u32::try_from(index).ok())
            .map(ProgramResourceId)
    }
}
