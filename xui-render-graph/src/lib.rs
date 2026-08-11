//! Backend-independent compilation of layer effects and backdrop filters.
//!
//! [`compile_layer`] normalizes static style into a reusable [`LayerProgram`].
//! [`LayerProgram::instantiate`] applies frame geometry, lowers executable pass IR,
//! and assigns abstract transient texture slots. The crate has no GPU dependency.

#![forbid(unsafe_code)]

mod binding;
mod compiler;
mod error;
mod input;
mod matrix;
mod plan;
mod program;

pub use binding::{BindingError, BoundLayerProgram, ExternalBindings, LayerMaskBinding};
pub use compiler::compile_layer;
pub use error::{CompileError, PlanError};
pub use input::{
    BackdropDescriptor, BackdropFilter, BlendMode, ColorMatrix, CompositeDescriptor,
    CompositeInstance, CompositeOperator, FilterQuality, LayerEffect, LayerGraphDescriptor, Mask,
    MaskShape, WorkingColorSpace,
};
pub use plan::{
    AttachmentBlend, Axis, CoordinateSpace, DrawProgram, DrawShader, Extent2d, ExternalAliasing,
    LayerPlanContext, LayerProgramEntry, LayerRenderPlan, Pass, PassId, PassOp, PassUniforms,
    PipelineKey, PixelRect, PlanLimits, PlanMask, PlanResource, PlanResourceId, PlanResourceKind,
    PlanStats, ResourceBindings, TextureClass, TransientSlot, TransientSlotId,
};
pub use program::{
    ExternalResourceKind, LayerProgram, MaskProgram, ProgramFingerprint, ProgramNode,
    ProgramNodeId, ProgramOp, ProgramResource, ProgramResourceId, ProgramResourceKind,
    SampleExpansion,
};
