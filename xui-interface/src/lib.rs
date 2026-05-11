pub mod core;
pub mod dirty;
pub mod event;
pub mod render;
pub mod runtime;
pub mod widget;

pub use core::{Color, EdgeInsets, Point, Rect, Size};
pub use dirty::DirtyFlags;
pub use event::{Event, EventContext, EventPhase, EventResult, Key as InputKey, PointerButton};
pub use render::{
    DamageRegion, DrawBackend, FontRenderBackend, MockRenderBackend, PaintCommand, Painter,
    RenderBackend,
};
pub use runtime::EventSource;
pub use widget::{Key, NodeId, NodeType, TextMeasurer, Widget, WidgetKind};
