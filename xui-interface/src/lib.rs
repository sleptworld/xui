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
pub mod dirty;
pub mod events;
pub mod platform;
pub mod render;
pub mod runtime;
pub mod state;
pub mod style;
pub mod text;
pub mod transition;
pub mod widget;

pub use core::{Bounds, Color, EdgeInsets, Point, Rect, Size, Sizing, Translation};
pub use dirty::{StyleDiffFlags, WidgetUpdateFlags};
pub use events::{
    CommandEvent, CommandId, ContextMenuTrigger, Event, EventPhase, EventRef, EventRequest,
    EventRequests, EventResult, FocusReason, KeyState, KeyText, Modifiers, NamedKey, PhysicalKey,
    PointerButton, PointerButtons, PointerCoords, PointerKind, PointerSnapshot, RawContextMenu,
    RawIme, RawKeyInput, RawKeyboard, RawPointerButton, RawPointerCancel, RawPointerMove, RawWheel,
    RawWindowEvent, ScrollDelta, Shortcut, ShortcutBinding, ShortcutKey, ShortcutModifiers,
    TextPayload, XuiDeviceId, XuiPointerId,
};
pub use platform::{PlatformOutput, TextInputPurpose, TextInputSession};
pub use render::*;
pub use runtime::EventSource;
pub use style::{
    AlignStyle, BackdropFilter, BackdropMask, BackdropStyle, BlendMode, ColorMatrix, ColorStyle,
    ColorToken, ColorValue, ComputedBackdropFilter, ComputedBackdropMask, ComputedBackdropStyle,
    ComputedColorStyle, ComputedEffect, ComputedEffectStyle, ComputedLayoutStyle,
    ComputedLinearGradientStyle, ComputedMaskShape, ComputedPaintStyle,
    ComputedRadialGradientStyle, ComputedScrollStyle, ComputedScrollbarStyle, ComputedShadowStyle,
    ComputedStrokeStyle, ComputedStyle, ComputedTextStyle, Effect, EffectStylePatch, FilterQuality,
    FlexDirectionStyle, FontSizeToken, JustifyStyle, LayoutStylePatch, LengthValue,
    LinearGradientStyle, MaskShape, PaintStylePatch, PositionStyle, RadialGradientStyle,
    RadiusToken, ScrollDirectionStyle, ScrollStylePatch, ScrollbarStyle, ScrollbarStylePatch,
    CursorIcon, ScrollbarVisibilityStyle, ShadowStyle, SpacingToken, StateStyleRule, Stroke, StrokeLineStyle,
    StrokeStyle, Style, StyleMerge, StylePatch, StyleValue, TextStylePatch, Theme, TransformStyle,
    TransformStylePatch, WidgetStateMatcher,
};
pub use text::*;
pub use transition::{AnimationProgress, Easing, Transition};
pub use widget::{
    AccessibilityProperties, AccessibilityRole, Component, FocusProperties, Focusability, Key,
    NodeId, NodeLifecycleEvent, WidgetType,
};

pub use state::{WidgetNodeState, WidgetState};
