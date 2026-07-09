pub mod core;
pub mod dirty;
pub mod events;
pub mod render;
pub mod runtime;
pub mod state;
pub mod style;
pub mod text;
pub mod widget;

pub use core::{Color, EdgeInsets, Point, Rect, Size, Sizing, Translation};
pub use dirty::{StyleDiffFlags, WidgetUpdateFlags};
pub use events::{
    ContextMenuTrigger, Event, EventContext, EventPhase, EventRef, EventRequest, EventRequests,
    EventResult, Key as InputKey, Modifiers, PointerButton, PointerButtons, PointerCoords,
    PointerKind, PointerSnapshot, RawContextMenu, RawKey, RawPointerButton, RawPointerCancel,
    RawPointerMove, RawTextInput, RawWheel, RawWindowEvent, ScrollDelta, XuiDeviceId, XuiPointerId,
};
pub use render::*;
pub use runtime::EventSource;
pub use style::{
    AlignStyle, ColorStyle, ColorToken, ColorValue, ComputedColorStyle, ComputedLayoutStyle,
    ComputedLinearGradientStyle, ComputedPaintStyle, ComputedRadialGradientStyle,
    ComputedScrollStyle, ComputedScrollbarStyle, ComputedShadowStyle, ComputedStrokeStyle,
    ComputedStyle, ComputedTextStyle, FlexDirectionStyle, FontSizeToken, JustifyStyle,
    LayoutStylePatch, LengthValue, LinearGradientStyle, PaintStylePatch, RadialGradientStyle,
    RadiusToken, ScrollDirectionStyle, ScrollStylePatch, ScrollbarStyle, ScrollbarStylePatch,
    ScrollbarVisibilityStyle, ShadowStyle, SpacingToken, StateStyleRule, Stroke, StrokeLineStyle,
    StrokeStyle, Style, StyleMerge, StylePatch, StyleValue, TextStylePatch, Theme,
    WidgetStateMatcher,
};
pub use text::*;
pub use widget::{Component, Key, NodeId, NodeLifecycleEvent, Widget, WidgetType};

pub use state::{WidgetNodeState, WidgetState};
