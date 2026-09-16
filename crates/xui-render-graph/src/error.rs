/// Errors produced while normalizing a static layer descriptor.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CompileError {
    #[error("invalid style field `{field}`: {reason}")]
    InvalidStyleParameter {
        field: &'static str,
        reason: &'static str,
    },
    #[error("invalid filter {index} field `{field}`: {reason}")]
    InvalidFilterParameter {
        index: usize,
        field: &'static str,
        reason: &'static str,
    },
    #[error("invalid layer effect {index} field `{field}`: {reason}")]
    InvalidEffectParameter {
        index: usize,
        field: &'static str,
        reason: &'static str,
    },
    #[error("the layer program contains more than u32::MAX nodes or resources")]
    TooManyNodes,
}

/// Errors produced while instantiating a program for frame geometry.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("invalid plan context field `{field}`: {reason}")]
    InvalidContext {
        field: &'static str,
        reason: &'static str,
    },
    #[error("logical bounds cannot be represented as physical pixel coordinates")]
    CoordinateOverflow,
    #[error("texture extent {width}x{height} exceeds the {limit} pixel limit")]
    TextureTooLarge { width: u32, height: u32, limit: u32 },
    #[error("the render plan contains more than u32::MAX passes, resources, or slots")]
    TooManyItems,
    #[error("internal render-plan invariant failed: {0}")]
    InternalInvariant(&'static str),
}
