//! Shared type vocabulary for the `xui` framework.
//!
//! Zero-runtime contracts every other crate agrees on: geometry (`Color`,
//! `Point`, `Rect`, `Size`, `Sizing`, `EdgeInsets`), the style system (`Style`,
//! `StylePatch`, `ComputedStyle`, tokens, themes, state matchers), input and
//! semantic events, widget contracts (`Component`, `NodeId`, `Key`), platform
//! integration (`TextInputSession`), animation timing (`Easing`, `Transition`),
//! text traits and layout data, and dirty-tracking flags. Has no dependency on
//! the runtime or any backend, which keeps the type surface stable.
//!
//! Modules: `core`, `style`, `events`, `widget`, `platform`, `transition`,
//! `text`, `dirty`, `runtime`, `render`. All items are re-exported at the crate
//! root for convenience.

pub mod core;

/// Ordered `f64`, as accessibility values carry it.
pub use ordered_float::OrderedFloat;
pub mod dirty;
pub mod events;
pub mod platform;
pub mod render;
pub mod runtime;
pub mod state;
pub mod style;
pub mod styles;
pub mod text;
pub mod transition;
pub mod widget;

pub use core::{Bounds, Color, EdgeInsets, Point, Rect, Size, Sizing, Translation};
pub use dirty::{StyleDiffFlags, WidgetUpdateFlags};
#[allow(deprecated)]
pub use events::{
    CommandEvent, CommandId, ContextMenuTrigger, Event, EventPhase, EventRef, EventRequest,
    EventRequests, EventResult, FocusReason, GestureKind, GesturePhase, KeyState, KeyText,
    Modifiers, NamedKey, PhysicalKey, PlatformReplyEvent, PointerBoundaryEvent, PointerButton,
    PointerButtons, PointerCoords, PointerKind, PointerMoveEvent, PointerSnapshot, RawContextMenu,
    RawGesture, RawIme, RawKeyInput, RawKeyboard, RawPointerButton, RawPointerCancel,
    RawPointerLeave, RawPointerMove, RawWheel, RawWindowEvent, ScrollDelta, Shortcut,
    ShortcutBinding, ShortcutKey, ShortcutModifiers, SwipeDirection, TextPayload, WakeEvent,
    XuiDeviceId, XuiPointerId,
};
pub use platform::{
    Appearance, ClipboardContent, ClipboardTarget, ColorScheme, DragEvent, DragPayload,
    DropAccepts, DropEffect, DropTarget, FileDialog, FileFilter, HostCapabilities, Menu, MenuBar,
    MenuEntry, MenuItem, MenuRole, MessageBox, MessageLevel, PlatformCommand, PlatformError,
    PlatformOutput, PlatformReply, PlatformRequest, PublishedSurface, RegionRect, RequestId,
    ResizeEdge, TextDocument, TextInputPurpose, TextInputSession, WindowCommand, WindowGeometry,
    WindowRegion, WindowState, commands,
};
pub use render::*;
pub use runtime::EventSource;
pub use style::{
    AlignStyle, BackdropFilter, BackdropMask, BackdropStyle, BlendMode, ColorMatrix, ColorStyle,
    ColorToken, ColorValue, ComputedBackdropFilter, ComputedBackdropMask, ComputedBackdropStyle,
    ComputedColorStyle, ComputedEffect, ComputedEffectStyle, ComputedLayoutStyle,
    ComputedLinearGradientStyle, ComputedMaskShape, ComputedPaintStyle,
    ComputedRadialGradientStyle, ComputedScrollStyle, ComputedScrollbarStyle, ComputedShadowStyle,
    ComputedStrokeStyle, ComputedStyle, ComputedTextStyle, CursorIcon, DisplayStyle, Effect,
    EffectStylePatch, FilterQuality, FlexDirectionStyle, FontSizeToken, JustifyStyle,
    LayoutStylePatch, LengthValue, LinearGradientStyle, MaskShape, PaintStylePatch, PositionStyle,
    RadialGradientStyle, RadiusToken, ScrollDirectionStyle, ScrollStylePatch, ScrollbarStyle,
    ScrollbarStylePatch, ScrollbarVisibilityStyle, ShadowStyle, SpacingToken, StateStyleRule,
    Stroke, StrokeLineStyle, StrokeStyle, Style, StyleMerge, StylePatch, StyleValue,
    TextStylePatch, Theme, TransformStyle, TransformStylePatch, VisibilityStyle,
    WidgetStateMatcher,
};
pub use text::*;
pub use transition::{
    Animation, AnimationDirection, AnimationProgress, Easing, Iterations, Transition,
};
pub use widget::{
    AccessibilityChecked, AccessibilityLiveRegion, AccessibilityOrientation, AccessibilityPopup,
    AccessibilityProperties, AccessibilityRole, Component, FocusProperties, Focusability, Key,
    NodeId, NodeLifecycleEvent, WidgetType,
};

pub use state::{WidgetNodeState, WidgetState};
