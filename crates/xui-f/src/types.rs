/// Stable identifier for a face registered in an `FBackend`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FFontId(pub(crate) u32);

impl FFontId {
    pub const fn raw(self) -> u32 {
        self.0
    }
}
