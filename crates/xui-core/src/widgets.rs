//! The built-in host widgets, and how the runtime holds them.
//!
//! Each widget is a node definition (`#[widget]`) and a `WidgetT` impl: its
//! props, its state, the capabilities it selects. While mounted, its own data
//! is a [`WidgetInstance`] in the [`Widgets`] enum, and its capabilities' rows
//! live in the runtime's columns. What a widget's own code does — paint its
//! content, shape its text, react to events — are its roles ([`roles`]).

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use xui_interface::core::Bounds;
use xui_interface::{EventRef, EventResult, NodeId, Size, TextContent, TextProps};
use xui_interface::{WidgetUpdateFlags, style::ComputedStyle};

use crate::capability::CommitCx;
use crate::caps::{CapMask, Columns};
use xui_interface::Key;
use crate::node::{Draft, HostNode, WidgetInstance, WidgetT};
use crate::render::RenderTreeWriter;

pub use crate::event_system::callbacks::EventHandlers;

/// A write-only own property of a widget: a shorthand setting several of its
/// props at once, listed in the widget's property table. Accepts anything
/// `Into` its value.
macro_rules! shorthand {
    (
        $(#[$meta:meta])* $name:ident : $widget:ty => $table:ty, $value:ty,
        |$props:ident, $v:ident| $body:expr
    ) => {
        $(#[$meta])*
        pub enum $name {}

        impl $table {
            #[doc(hidden)]
            #[inline(always)]
            pub fn $name(&self) -> crate::property::PropertyMarker<$name> {
                crate::property::PropertyMarker::new()
            }
        }

        impl crate::property::Property for $name {
            type Scope = crate::node::OwnProps<$widget>;
            type Requirement = crate::property::NotRequired;
            type Value = $value;

            fn set($props: &mut <$widget as crate::node::NodeDef>::Props, $v: $value) {
                $body;
            }
        }

        impl<T: Into<$value>> crate::property::Accepts<T> for $name {
            fn accept(value: T) -> $value {
                value.into()
            }
        }
    };
}

pub(crate) mod canvas;
pub(crate) mod container;
pub(crate) mod grid;
pub(crate) mod icon;
pub(crate) mod image;
pub(crate) mod overlay;
pub(crate) mod roles;
pub(crate) mod root;
pub(crate) mod text;
pub(crate) mod text_input;
pub(crate) mod z_stack;

pub(crate) use canvas::CanvasInvalidator;
#[cfg(feature = "wgpu")]
pub use canvas::{CANVAS_GPU_FORMAT, CANVAS_GPU_USAGES, CanvasGpuPainter, GpuPainter};
pub use canvas::{
    CanvasContent, CanvasController, CanvasGpuContext, CanvasPainter, CanvasPick, CanvasPickTag,
    CanvasTextLayout, CanvasTextMetrics,
};
pub(crate) use canvas::{CanvasFrame, canvas_text_slot};
pub(crate) use grid::grid_widget_to_taffy;
pub use grid::{GridFlow, GridTrackSize, GridTracks};
pub use icon::{IconData, IconLayer, IconStroke, SvgIconError};
pub use overlay::{
    DismissHandler, DismissReason, OverlayChild, OverlayEntry, OverlayEntryId, OverlayEntryOptions,
    OverlayModelError, OverlayScope, OverlayScopeId,
};
#[doc(hidden)]
pub use text_input::keymap::{TextCommand, TextKeymap};
pub(crate) use text_input::text_input_content_box;
pub use text_input::value::TextSelection;
pub use text_input::{TextController, TextInputChange};

use canvas::Canvas;
use container::Container;
use grid::Grid;
use icon::Icon;
use image::Image;
use overlay::RootOverlayer;
use root::Root;
use text::Text;
use text_input::TextInput;
use z_stack::ZStack;

/// A widget's place in [`Widgets`].
pub trait WidgetVariant: WidgetT {
    const NODE_TYPE: WidgetType;

    fn wrap(instance: WidgetInstance<Self>) -> Widgets;
    fn unwrap(widgets: &Widgets) -> Option<&WidgetInstance<Self>>;
    fn unwrap_mut(widgets: &mut Widgets) -> Option<&mut WidgetInstance<Self>>;
}

macro_rules! define_widgets {
    (@role $at:tt paint) => {
        fn as_paint(&self) -> Option<&dyn roles::PaintContent> {
            Some(define_widgets!(@at self $at))
        }
    };
    (@role $at:tt text) => {
        fn as_text(&self) -> Option<&dyn roles::TextSource> {
            Some(define_widgets!(@at self $at))
        }
    };
    (@role $at:tt events) => {
        fn as_handle_event(&self) -> Option<&dyn roles::HandleEvent> {
            Some(define_widgets!(@at self $at))
        }

        fn as_handle_event_mut(&mut self) -> Option<&mut dyn roles::HandleEvent> {
            Some(define_widgets!(@at_mut self $at))
        }
    };
    (@role $at:tt text_input) => {
        fn as_text_input(&self) -> Option<&dyn roles::PlatformTextInput> {
            Some(define_widgets!(@at self $at))
        }
    };
    (@role $at:tt intrinsic_size) => {
        fn as_intrinsic_size(&self) -> Option<&dyn roles::IntrinsicSize> {
            Some(define_widgets!(@at self $at))
        }
    };
    (@role $at:tt arrange) => {
        fn as_arrange(&self) -> Option<&dyn roles::ArrangeChildren> {
            Some(define_widgets!(@at self $at))
        }
    };
    (@role $at:tt default_focus) => {
        fn as_default_focus(&self) -> Option<&dyn roles::DefaultFocus> {
            Some(define_widgets!(@at self $at))
        }
    };
    (@entry $widget:ident [$($role:ident),*] $at:tt) => {
        impl roles::WidgetRoles for WidgetInstance<$widget> {
            $(define_widgets!(@role $at $role);)*
        }
    };
    // Roles are implemented on the instance, or on its state (`via state`).
    (@at $self:ident []) => { $self };
    (@at $self:ident [state]) => { &$self.state };
    (@at_mut $self:ident []) => { $self };
    (@at_mut $self:ident [state]) => { &mut $self.state };

    ($($name:ident => $widget:ident [$($role:ident),* $(,)?] $(via $at:ident)?),+ $(,)?) => {
        /// Which host widget a node is.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum WidgetType {
            $($name,)+
        }

        /// Every mounted host widget's own data, by widget.
        #[derive(Debug)]
        pub enum Widgets {
            $($name(WidgetInstance<$widget>),)+
        }

        $(
            impl WidgetVariant for $widget {
                const NODE_TYPE: WidgetType = WidgetType::$name;

                fn wrap(instance: WidgetInstance<Self>) -> Widgets {
                    Widgets::$name(instance)
                }

                fn unwrap(widgets: &Widgets) -> Option<&WidgetInstance<Self>> {
                    match widgets {
                        Widgets::$name(instance) => Some(instance),
                        #[allow(unreachable_patterns)]
                        _ => None,
                    }
                }

                fn unwrap_mut(widgets: &mut Widgets) -> Option<&mut WidgetInstance<Self>> {
                    match widgets {
                        Widgets::$name(instance) => Some(instance),
                        #[allow(unreachable_patterns)]
                        _ => None,
                    }
                }
            }

        )+

        $(define_widgets!(@entry $widget [$($role),*] [$($at)?]);)+

        impl Widgets {
            pub fn node_type(&self) -> WidgetType {
                match self {
                    $(Self::$name(_) => WidgetType::$name,)+
                }
            }

            /// The capabilities the widget selects.
            pub(crate) fn caps(&self) -> CapMask {
                match self {
                    $(Self::$name(_) => <$widget as HostNode>::CAPS,)+
                }
            }

            /// The widget's declared roles.
            pub(crate) fn roles(&self) -> &dyn roles::WidgetRoles {
                match self {
                    $(Self::$name(instance) => instance,)+
                }
            }

            pub(crate) fn roles_mut(&mut self) -> &mut dyn roles::WidgetRoles {
                match self {
                    $(Self::$name(instance) => instance,)+
                }
            }
        }
    };
}

// Each widget's roles (see `roles`): the behaviour its own code implements,
// on its instance or, `via state`, on its state. A role listed here that the
// widget does not implement fails to compile.
define_widgets! {
    Container => Container [arrange],
    ZStack => ZStack [arrange],
    Text => Text [paint, text],
    TextInput => TextInput [paint, text, events, text_input, default_focus] via state,
    Canvas => Canvas [paint] via state,
    Image => Image [paint, intrinsic_size],
    Icon => Icon [paint],
    Grid => Grid [arrange],
    RootOverlayer => RootOverlayer [],
    Root => Root [arrange],
}

impl Widgets {
    /// Whether it reads `EventRef::Raw` — see [`roles::HandleEvent::raw_events`].
    pub(crate) fn reads_raw_events(&self) -> bool {
        self.roles()
            .as_handle_event()
            .is_some_and(|it| it.raw_events())
    }

    /// Paints the widget's own content, if it has any.
    pub(crate) fn render(
        &self,
        node_id: NodeId,
        rect: Bounds,
        style: &ComputedStyle,
        writer: &mut RenderTreeWriter<'_>,
    ) {
        if let Some(content) = self.roles().as_paint() {
            content.paint(node_id, rect, style, writer);
        }
    }

    pub fn handle_event(
        &mut self,
        event: EventRef<'_>,
        cx: &mut crate::event_system::EventContext<'_>,
    ) -> EventResult {
        self.roles_mut()
            .as_handle_event_mut()
            .map_or(EventResult::Ignored, |it| it.handle_event(event, cx))
    }

    pub fn text(&self) -> Option<TextContent> {
        self.roles().as_text().map(|it| it.text())
    }

    pub fn text_layout_props(&self, style: &ComputedStyle) -> Option<TextProps> {
        self.roles().as_text().map(|it| it.text_layout(style))
    }

    pub(crate) fn platform_text_input_session(
        &self,
        content_rect: Bounds,
        text_layout: &dyn crate::text::TextLayoutQuery,
    ) -> Option<xui_interface::TextInputSession> {
        self.roles()
            .as_text_input()
            .map(|it| it.text_input_session(content_rect, text_layout))
    }

    pub fn intrinsic_size(&self) -> Option<Size<f32>> {
        self.roles()
            .as_intrinsic_size()
            .and_then(|it| it.intrinsic_size())
    }

    /// Whether it takes focus when its focusability is `Auto`.
    pub(crate) fn focusable_by_default(&self) -> bool {
        self.roles()
            .as_default_focus()
            .is_some_and(|it| it.focusable())
    }
}

/// A host element as written: a [`Draft`] of some widget, its type erased
/// until it mounts or updates the node it describes.
pub(crate) trait ErasedDraft {
    fn node_type(&self) -> WidgetType;

    fn key(&self) -> Option<&Key>;

    /// Spreads the draft over node `id`'s rows and builds its instance.
    fn mount(
        self: Box<Self>,
        id: NodeId,
        columns: &mut Columns,
        cx: &mut CommitCx<'_>,
    ) -> (Widgets, WidgetUpdateFlags);

    /// Spreads the draft over `id`'s rows and moves `current` to its props;
    /// returns what changed.
    fn update(
        self: Box<Self>,
        current: &mut Widgets,
        id: NodeId,
        columns: &mut Columns,
        cx: &mut CommitCx<'_>,
    ) -> WidgetUpdateFlags;
}

impl<N> ErasedDraft for Draft<N>
where
    N: WidgetVariant,
    N::Props: Clone,
    N::Behavior: Clone,
{
    fn node_type(&self) -> WidgetType {
        N::NODE_TYPE
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn mount(
        self: Box<Self>,
        id: NodeId,
        columns: &mut Columns,
        cx: &mut CommitCx<'_>,
    ) -> (Widgets, WidgetUpdateFlags) {
        let (props, flags) = N::scatter(*self, id, columns, cx);
        let state = N::create_state(&props);
        (N::wrap(WidgetInstance { props, state }), flags)
    }

    fn update(
        self: Box<Self>,
        current: &mut Widgets,
        id: NodeId,
        columns: &mut Columns,
        cx: &mut CommitCx<'_>,
    ) -> WidgetUpdateFlags {
        let instance = N::unwrap_mut(current).expect("a draft updates only its own widget");
        let (props, mut flags) = N::scatter(*self, id, columns, cx);
        flags |= N::props_dirty(&instance.props, &props);
        let old = std::mem::replace(&mut instance.props, props);
        flags | N::update(&old, &instance.props, &mut instance.state)
    }
}

impl std::fmt::Debug for dyn ErasedDraft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Draft")
            .field("widget", &self.node_type())
            .field("key", &self.key())
            .finish_non_exhaustive()
    }
}

/// A mounted widget's own data, shared between the host tree and whatever
/// borrows it for a moment (paint, event dispatch).
#[derive(Debug, Clone)]
pub struct WidgetI {
    widget: Rc<RefCell<Widgets>>,
}

impl WidgetI {
    pub(crate) fn new(widget: Widgets) -> Self {
        Self {
            widget: Rc::new(RefCell::new(widget)),
        }
    }

    pub(crate) fn with_widgets<R>(&self, f: impl FnOnce(&Widgets) -> R) -> R {
        f(&self.widget.borrow())
    }

    pub(crate) fn with_widgets_mut<R>(&self, f: impl FnOnce(&mut Widgets) -> R) -> R {
        f(&mut self.widget.borrow_mut())
    }

    pub fn node_type(&self) -> WidgetType {
        self.with_widgets(Widgets::node_type)
    }

    pub(crate) fn caps(&self) -> CapMask {
        self.with_widgets(Widgets::caps)
    }

    pub(crate) fn reads_raw_events(&self) -> bool {
        self.with_widgets(Widgets::reads_raw_events)
    }

    pub fn text(&self) -> Option<TextContent> {
        self.with_widgets(Widgets::text)
    }

    pub(crate) fn platform_text_input_session(
        &self,
        content_rect: Bounds,
        text_layout: &dyn crate::text::TextLayoutQuery,
    ) -> Option<xui_interface::TextInputSession> {
        self.with_widgets(|widget| widget.platform_text_input_session(content_rect, text_layout))
    }

    pub fn intrinsic_size(&self) -> Option<Size<f32>> {
        self.with_widgets(Widgets::intrinsic_size)
    }

    pub(crate) fn focusable_by_default(&self) -> bool {
        self.with_widgets(Widgets::focusable_by_default)
    }

    pub(crate) fn render(
        &self,
        node_id: NodeId,
        rect: Bounds,
        style: &ComputedStyle,
        writer: &mut RenderTreeWriter<'_>,
    ) {
        self.with_widgets(|widget| widget.render(node_id, rect, style, writer));
    }

    pub fn handle_event(
        &self,
        event: EventRef<'_>,
        cx: &mut crate::event_system::EventContext<'_>,
    ) -> EventResult {
        self.with_widgets_mut(|widget| widget.handle_event(event, cx))
    }
}

/// The root of every host tree.
pub(crate) fn root_widget() -> Draft<Root> {
    root::root()
}

/// The layer portals mount under.
pub(crate) fn root_overlayer_widget() -> Draft<RootOverlayer> {
    overlay::root_overlayer()
}

pub(super) fn props_hash<T: Hash>(props: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    props.hash(&mut hasher);
    hasher.finish()
}
