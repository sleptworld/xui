pub mod core;
pub mod dirty;
pub mod event;
pub mod render;
pub mod runtime;
pub mod text;
pub mod widget;

pub use core::{Color, EdgeInsets, Point, Rect, Size};
pub use dirty::DirtyFlags;
pub use event::{
    ClickEventHandler, Event, EventContext, EventHandler, EventHandlers, EventPhase, EventRequest,
    EventRequests, EventResult, HoverChangeEventHandler, Key as InputKey, KeyEventHandler,
    PointerButton, PointerEventHandler,
};
pub use render::{
    DamageRegion, DrawBackend, FontRenderBackend, MockRenderBackend, PaintCommand, Painter,
    RenderBackend,
};
pub use runtime::EventSource;
pub use text::{
    FontFamily, FontStyle, FontWeight, LineHeight, OverflowWrap, ParagraphStyle, TextAlign,
    TextBoxStyle, TextContent, TextDecoration, TextOverflow, TextProps, TextStyle,
    TextVerticalAlign, WhiteSpace,
};
pub use widget::{Component, Key, NodeId, TextMeasurer, Widget, WidgetType};
