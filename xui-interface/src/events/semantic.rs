use super::raw::{
    ContextMenuTrigger, Modifiers, PointerButton, PointerSnapshot, RawEvent, ScrollDelta,
};
use super::shortcut::{CommandId, Shortcut};
use crate::core::{Point, Translation};
use crate::widget::NodeId;
use std::time::{Duration, Instant};

pub type EventId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventPhase {
    Capture,
    Target,
    Bubble,
}

#[derive(Debug, Clone)]
pub enum XuiEvent {
    Raw(RawEvent),
    Semantic(SemanticEvent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropagationMode {
    Direct,
    CaptureTarget,
    CaptureTargetBubble,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventSource {
    Pointer,
    Keyboard,
    Scroll,
    Programmatic,
    Window,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PointerId(pub u64);

#[derive(Debug, Clone)]
pub struct EventMeta {
    pub id: EventId,
    pub timestamp: Instant,
    pub target: NodeId,
    pub current_target: NodeId,
    pub phase: EventPhase,
    pub source: EventSource,
    pub modifiers: Modifiers,
}
#[derive(Debug, Clone)]
pub enum SemanticEvent {
    Command(CommandEvent),
    // Hover
    /// This node entered or left the hover path. See [`HoverEvent`].
    Hovered(HoverEvent),
    // Press
    PressStart(PressEvent),
    PressEnd(PressEvent),
    PressCancel(PressEvent),
    // Click
    Click(ClickEvent),
    DoubleClick(ClickEvent),
    ContextMenu(ContextMenuEvent),

    Focus(FocusEvent),
    Blur(FocusEvent),
    FocusIn(FocusEvent),
    FocusOut(FocusEvent),

    DragStart(DragEvent),
    DragMove(DragEvent),
    DragEnd(DragEvent),
    DragCancel(DragEvent),

    Scroll(ScrollEvent),
}

impl SemanticEvent {
    pub fn meta(&self) -> &EventMeta {
        match self {
            SemanticEvent::Command(e) => &e.meta,
            SemanticEvent::Hovered(e) => &e.meta,

            SemanticEvent::PressStart(e) => &e.meta,
            SemanticEvent::PressEnd(e) => &e.meta,
            SemanticEvent::PressCancel(e) => &e.meta,

            SemanticEvent::Click(e) => &e.meta,
            SemanticEvent::DoubleClick(e) => &e.meta,
            SemanticEvent::ContextMenu(e) => &e.meta,

            SemanticEvent::Focus(e) => &e.meta,
            SemanticEvent::Blur(e) => &e.meta,
            SemanticEvent::FocusIn(e) => &e.meta,
            SemanticEvent::FocusOut(e) => &e.meta,

            SemanticEvent::DragStart(e) => &e.meta,
            SemanticEvent::DragMove(e) => &e.meta,
            SemanticEvent::DragEnd(e) => &e.meta,
            SemanticEvent::DragCancel(e) => &e.meta,

            SemanticEvent::Scroll(e) => &e.meta,
        }
    }

    pub fn meta_mut(&mut self) -> &mut EventMeta {
        match self {
            SemanticEvent::Command(e) => &mut e.meta,
            SemanticEvent::Hovered(e) => &mut e.meta,

            SemanticEvent::PressStart(e) => &mut e.meta,
            SemanticEvent::PressEnd(e) => &mut e.meta,
            SemanticEvent::PressCancel(e) => &mut e.meta,

            SemanticEvent::Click(e) => &mut e.meta,
            SemanticEvent::DoubleClick(e) => &mut e.meta,
            SemanticEvent::ContextMenu(e) => &mut e.meta,

            SemanticEvent::Focus(e) => &mut e.meta,
            SemanticEvent::Blur(e) => &mut e.meta,
            SemanticEvent::FocusIn(e) => &mut e.meta,
            SemanticEvent::FocusOut(e) => &mut e.meta,

            SemanticEvent::DragStart(e) => &mut e.meta,
            SemanticEvent::DragMove(e) => &mut e.meta,
            SemanticEvent::DragEnd(e) => &mut e.meta,
            SemanticEvent::DragCancel(e) => &mut e.meta,

            SemanticEvent::Scroll(e) => &mut e.meta,
        }
    }

    pub fn propagation_mode(&self) -> PropagationMode {
        match self {
            SemanticEvent::Command(_) => PropagationMode::CaptureTargetBubble,
            // Direct on purpose. The translator diffs the whole hover path and
            // emits one event per node that actually changed, so every hovered
            // ancestor is already reached. Bubbling would deliver a second copy
            // of a child's event to ancestors that got their own.
            SemanticEvent::Hovered(_) => PropagationMode::Direct,

            SemanticEvent::PressStart(_) => PropagationMode::CaptureTargetBubble,
            SemanticEvent::PressEnd(_) => PropagationMode::CaptureTargetBubble,
            SemanticEvent::PressCancel(_) => PropagationMode::CaptureTargetBubble,

            SemanticEvent::Click(_) => PropagationMode::CaptureTargetBubble,
            SemanticEvent::DoubleClick(_) => PropagationMode::CaptureTargetBubble,
            SemanticEvent::ContextMenu(_) => PropagationMode::CaptureTargetBubble,

            SemanticEvent::Focus(_) => PropagationMode::Direct,
            SemanticEvent::Blur(_) => PropagationMode::Direct,

            SemanticEvent::FocusIn(_) => PropagationMode::CaptureTargetBubble,
            SemanticEvent::FocusOut(_) => PropagationMode::CaptureTargetBubble,

            SemanticEvent::DragStart(_) => PropagationMode::CaptureTargetBubble,
            SemanticEvent::DragMove(_) => PropagationMode::CaptureTargetBubble,
            SemanticEvent::DragEnd(_) => PropagationMode::CaptureTargetBubble,
            SemanticEvent::DragCancel(_) => PropagationMode::CaptureTargetBubble,

            SemanticEvent::Scroll(_) => PropagationMode::CaptureTargetBubble,
        }
    }

    pub fn bubbles(&self) -> bool {
        matches!(
            self.propagation_mode(),
            PropagationMode::CaptureTargetBubble
        )
    }

    pub fn captures(&self) -> bool {
        matches!(
            self.propagation_mode(),
            PropagationMode::CaptureTarget | PropagationMode::CaptureTargetBubble
        )
    }
}

#[derive(Debug, Clone)]
pub struct CommandEvent {
    pub meta: EventMeta,
    pub command: CommandId,
    pub shortcut: Shortcut,
}

/// A node entered or left the hover path.
///
/// Delivered once to each node whose hover state actually changed, which
/// includes every ancestor that gained or lost hover — the translator diffs the
/// old and new root-to-target paths rather than reporting only the leaf.
///
/// # Prefer styling to handling
///
/// If all you want is to *look* different while hovered, do not use this event.
/// `style!(background: if hovered { .. })` lowers to a `WidgetStateMatcher` that
/// the style system resolves without re-rendering the component, and it can be
/// animated by the transition system. Handling this event and storing the result
/// in state turns every pointer move across a boundary into a component render.
///
/// Use it for side effects that styling cannot express: opening a tooltip after
/// a delay, prefetching, analytics.
#[derive(Debug, Clone)]
pub struct HoverEvent {
    pub meta: EventMeta,
    pub pointer: PointerSnapshot,
    /// `true` when this node just became hovered, `false` when it stopped.
    ///
    /// One event with a flag rather than a separate enter and leave event:
    /// consumers want the state, and splitting it made every one of them
    /// reconstruct this boolean from two handlers.
    pub hovered: bool,
    /// Where the pointer came from when entering, or went to when leaving.
    pub related_target: Option<NodeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PressId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressCancelReason {
    PointerLeft,
    PointerCaptureLost,
    WindowBlur,
    Disabled,
    Removed,
    ScrollStarted,
    AnotherPointerPressed,
    Programmatic,
}

#[derive(Debug, Clone)]
pub struct PressEvent {
    pub meta: EventMeta,
    pub press_id: PressId,
    pub pointer: PointerSnapshot,
    pub press_target: NodeId,
    pub start_position: Point,
    pub current_position: Point,
    pub delta: Translation,
    pub duration: Option<Duration>,
    pub cancel_reason: Option<PressCancelReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationKind {
    Pointer,
    Keyboard,
    Programmatic,
}

#[derive(Debug, Clone)]
pub struct ClickEvent {
    pub meta: EventMeta,
    pub activation: ActivationKind,
    pub pointer: Option<PointerSnapshot>,
    pub button: Option<PointerButton>,
    pub click_count: u8,
    pub press_target: Option<NodeId>,
    pub release_target: Option<NodeId>,
    pub duration: Option<Duration>,
}

#[derive(Debug, Clone)]
pub struct ContextMenuEvent {
    pub meta: EventMeta,
    pub trigger: ContextMenuTrigger,
    pub pointer: Option<PointerSnapshot>,
    pub position: Option<Point>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusReason {
    Pointer,
    Keyboard,
    Programmatic,
    Window,
    NodeRemoved,
    Disabled,
}

#[derive(Debug, Clone)]
pub struct FocusEvent {
    pub meta: EventMeta,

    /// 旧焦点节点
    pub old_focused: Option<NodeId>,

    /// 新焦点节点
    pub new_focused: Option<NodeId>,

    /// 对当前事件来说的关联节点。
    ///
    /// Focus:  related_target = old_focused
    /// Blur:   related_target = new_focused
    /// FocusIn / FocusOut 同理。
    pub related_target: Option<NodeId>,
    pub reason: FocusReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DragId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragCancelReason {
    PointerCaptureLost,
    WindowBlur,
    EscapePressed,
    SourceRemoved,
    Disabled,
    Programmatic,
}

#[derive(Debug, Clone)]
pub struct DragEvent {
    pub meta: EventMeta,

    pub drag_id: DragId,

    pub pointer: PointerSnapshot,

    /// drag 开始的节点
    pub source: NodeId,

    /// 当前指针下面的节点
    pub over: Option<NodeId>,

    /// drag 起点
    pub start_position: Point,

    /// 上一帧 / 上一次 drag move 的位置
    pub previous_position: Point,

    /// 当前指针位置
    pub current_position: Point,

    /// current - previous
    pub delta: Translation,

    /// current - start
    pub total_delta: Translation,

    /// DragEnd / DragCancel 时有意义
    pub duration: Option<Duration>,

    pub cancel_reason: Option<DragCancelReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollSource {
    Wheel,
    TouchPan,
    Trackpad,
    Keyboard,
    Scrollbar,
    Programmatic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollPhase {
    Start,
    Move,
    End,
    Momentum,
}

#[derive(Debug, Clone)]
pub struct ScrollEvent {
    pub meta: EventMeta,

    pub source: ScrollSource,

    pub phase: ScrollPhase,

    /// 原始滚动增量
    pub delta: ScrollDelta,

    /// 归一化之后的像素滚动量
    pub pixel_delta: Translation,

    /// 滚动容器节点
    pub scroll_target: NodeId,

    /// 滚动前 offset
    pub offset_before: Option<Translation>,

    /// 滚动后 offset
    pub offset_after: Option<Translation>,

    /// 实际被消费的滚动量
    pub consumed_delta: Option<Translation>,

    /// 没有被当前 scroll container 消费的剩余滚动量。
    /// 可以用于 scroll chaining。
    pub remaining_delta: Option<Translation>,

    /// 是否来自惯性滚动
    pub is_inertial: bool,

    /// 鼠标滚轮 / 触控板触发时有
    pub pointer: Option<PointerSnapshot>,
}
