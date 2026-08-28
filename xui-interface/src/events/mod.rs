pub mod raw;
pub mod semantic;
pub mod shortcut;
pub use raw::*;
pub use semantic::*;
pub use shortcut::*;

#[derive(Debug, Clone)]
pub enum Event {
    Raw(raw::RawEvent),
    Semantic(semantic::SemanticEvent),
}

#[derive(Debug, Clone, Copy)]
pub enum EventRef<'a> {
    Raw(&'a raw::RawEvent),
    Semantic(&'a semantic::SemanticEvent),
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventResult {
    Ignored,
    Consumed,
}

impl EventResult {
    pub fn is_consumed(self) -> bool {
        matches!(self, Self::Consumed)
    }
}
