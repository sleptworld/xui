#[derive(thiserror::Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    #[error("slot pointer does not belong to this scope")]
    Foreign,
    #[error("slot generation is stale")]
    Stale,
    #[error("slot is vacant")]
    Vacant,
    #[error("slot value has a different type")]
    TypeMismatch,
    #[error("slot generation overflowed")]
    GenerationOverflow,
}

pub type Result<T> = std::result::Result<T, Error>;
