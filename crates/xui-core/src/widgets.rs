use std::any::Any;
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

pub use crate::event_system::callbacks::EventHandlers;
use crate::event_system::interaction::{HostInteraction, InteractionProperties};
use xui_interface::core::Bounds;
use xui_interface::style::FlexDirectionStyle;
use xui_interface::{EventRef, EventResult, TextContent, TextProps, WidgetUpdateFlags};
pub use xui_interface::{Style, WidgetType};

mod utils;

use crate::core::Size;
use crate::element::{ComponentDesc, ElementDesc, WidgetDesc};
use crate::fiber::{ComponentRender, Key};
use crate::render::RenderTreeWriter;
use crate::state::HookContext;
use crate::style::ComputedStyle;

/// The interaction builder methods (focus, accessibility, shortcuts) of a
/// widget with `interaction` and `event_handlers` fields. Reading those fields
/// back is [`HostWidget::interaction`]'s job.
macro_rules! event_handler_methods {
    () => {
        pub fn focusable(mut self, focusable: bool) -> Self {
            self.interaction.focus = self.interaction.focus.focusable(focusable);
            self
        }

        pub fn tab_index(mut self, tab_index: i32) -> Self {
            self.interaction.focus = self.interaction.focus.tab_index(tab_index);
            self
        }

        pub fn focus_handle(mut self, handle: crate::focus::FocusHandle) -> Self {
            self.interaction.focus_handle = Some(handle);
            self
        }

        /// Observes this widget's laid-out border-box size without adding a
        /// wrapper or changing layout. Create the handle with `cx.use_size()`.
        pub fn observe_size(mut self, observer: crate::size::SizeObserver) -> Self {
            self.interaction.size_observer = Some(observer);
            self
        }

        /// Binds `controller` to this widget's scroll position. Only has an
        /// effect on a widget whose style makes it scrollable.
        pub fn scroll_controller(mut self, controller: crate::scroll::ScrollController) -> Self {
            self.interaction.scroll_controller = Some(controller);
            self
        }

        pub fn accessibility(
            mut self,
            accessibility: xui_interface::AccessibilityProperties,
        ) -> Self {
            self.interaction.accessibility = accessibility;
            self
        }

        pub fn accessibility_role(mut self, role: xui_interface::AccessibilityRole) -> Self {
            self.interaction.accessibility.role = Some(role);
            self
        }

        pub fn accessibility_id(mut self, id: impl Into<String>) -> Self {
            self.interaction.accessibility.id = Some(id.into());
            self
        }

        pub fn accessibility_label(mut self, label: impl Into<String>) -> Self {
            self.interaction.accessibility.label = Some(label.into());
            self
        }

        pub fn accessibility_description(mut self, description: impl Into<String>) -> Self {
            self.interaction.accessibility.description = Some(description.into());
            self
        }

        pub fn accessibility_selected(mut self, selected: bool) -> Self {
            self.interaction.accessibility.selected = Some(selected);
            self
        }

        pub fn accessibility_disabled(mut self, disabled: bool) -> Self {
            self.interaction.accessibility.disabled = Some(disabled);
            self
        }

        pub fn accessibility_controls(mut self, id: impl Into<String>) -> Self {
            self.interaction.accessibility.controls = Some(id.into());
            self
        }

        pub fn accessibility_labelled_by(mut self, id: impl Into<String>) -> Self {
            self.interaction.accessibility.labelled_by = Some(id.into());
            self
        }

        pub fn accessibility_checked(mut self, value: xui_interface::AccessibilityChecked) -> Self {
            self.interaction.accessibility.checked = Some(value);
            self
        }
        pub fn accessibility_orientation(
            mut self,
            value: xui_interface::AccessibilityOrientation,
        ) -> Self {
            self.interaction.accessibility.orientation = Some(value);
            self
        }
        pub fn accessibility_live_region(
            mut self,
            value: xui_interface::AccessibilityLiveRegion,
        ) -> Self {
            self.interaction.accessibility.live_region = Some(value);
            self
        }
        pub fn accessibility_expanded(mut self, value: bool) -> Self {
            self.interaction.accessibility.expanded = Some(value);
            self
        }
        pub fn accessibility_hidden(mut self, value: bool) -> Self {
            self.interaction.accessibility.hidden = Some(value);
            self
        }
        pub fn accessibility_required(mut self, value: bool) -> Self {
            self.interaction.accessibility.required = Some(value);
            self
        }
        pub fn accessibility_invalid(mut self, value: bool) -> Self {
            self.interaction.accessibility.invalid = Some(value);
            self
        }
        pub fn accessibility_read_only(mut self, value: bool) -> Self {
            self.interaction.accessibility.read_only = Some(value);
            self
        }
        pub fn accessibility_numeric_value(mut self, value: f64) -> Self {
            self.interaction.accessibility.numeric_value = Some(value.into());
            self
        }
        /// Numeric limits as one DSL argument.
        pub fn accessibility_value_range(mut self, range: (f64, f64, Option<f64>)) -> Self {
            self.interaction.accessibility.min_value = Some(range.0.into());
            self.interaction.accessibility.max_value = Some(range.1.into());
            self.interaction.accessibility.step_value = range.2.map(Into::into);
            self
        }
        /// Installs all key bindings in one `view!` modifier.
        pub fn shortcuts(
            mut self,
            bindings: impl IntoIterator<Item = (xui_interface::Shortcut, xui_interface::CommandId)>,
        ) -> Self {
            for (shortcut, command) in bindings {
                self = self.shortcut(shortcut, command);
            }
            self
        }
        pub fn shortcut(
            mut self,
            shortcut: xui_interface::Shortcut,
            command: xui_interface::CommandId,
        ) -> Self {
            if let Some(binding) = self
                .interaction
                .shortcuts
                .iter_mut()
                .find(|binding| binding.shortcut == shortcut)
            {
                binding.command = command;
            } else {
                self.interaction
                    .shortcuts
                    .push(xui_interface::ShortcutBinding { shortcut, command });
            }
            self
        }
    };
}

/// Opts a widget into the generated `on_*` vocabulary.
///
/// The 34 builder methods that used to be written out here now come from
/// `EventProps`, blanket-implemented over this trait — the same arrangement as
/// `Styled`/`StyleProps` on the style side.
macro_rules! impl_listen {
    ($($widget:ty),* $(,)?) => {
        $(
            impl crate::event_system::callbacks::Listen for $widget {
                fn handlers_mut(&mut self) -> &mut EventHandlers {
                    &mut self.event_handlers
                }
            }
        )*
    };
}

mod canvas;
mod container;
mod grid;
mod icon;
mod image;
mod overlay;
mod root;
mod text;
mod text_input;
mod z_stack;

pub(crate) use canvas::CanvasInvalidator;
#[cfg(feature = "wgpu")]
pub use canvas::{CANVAS_GPU_FORMAT, CANVAS_GPU_USAGES, CanvasGpuPainter, GpuPainter};
pub use canvas::{
    CanvasContent, CanvasController, CanvasGpuContext, CanvasPainter, CanvasPick, CanvasPickTag,
    CanvasTextLayout, CanvasTextMetrics, CanvasWidget,
};
pub(crate) use canvas::{CanvasFrame, canvas_text_slot};
pub use container::ContainerWidget;
pub use grid::*;
pub use icon::{IconData, IconLayer, IconStroke, IconWidget, SvgIconError};
pub use image::ImageWidget;
pub(crate) use overlay::RootOverlayerWidget;
pub use overlay::{
    DismissHandler, DismissReason, OverlayChild, OverlayEntry, OverlayEntryId, OverlayEntryOptions,
    OverlayModelError, OverlayScope, OverlayScopeId,
};
pub use root::RootWidget;
pub use text::TextWidget;
pub use text_input::TextInputWidget;
#[doc(hidden)]
pub use text_input::keymap::{TextCommand, TextKeymap};
pub(crate) use text_input::text_input_content_box;
pub use text_input::value::TextSelection;
pub use text_input::{TextController, TextInputChange};
pub use z_stack::ZStackWidget;

impl_listen!(
    CanvasWidget,
    ContainerWidget,
    GridWidget,
    IconWidget,
    ImageWidget,
    TextWidget,
    TextInputWidget,
    ZStackWidget,
);

pub type RootComponentRender = fn(&mut HookContext) -> ElementDesc;

/// What every host widget provides to the runtime.
///
/// [`Widgets`] dispatches each of its methods here, so a widget that leaves out
/// a required item fails to compile at its own `impl`, and one that has no use
/// for an optional item (it ignores events, has no text, takes no handlers)
/// keeps the default. Builder methods are not part of it: they stay inherent,
/// so what the DSL resolves `.attr(v)` to is unchanged.
pub(crate) trait HostWidget: Sized {
    /// What [`Widgets::node_type`] reports.
    const TYPE: WidgetType;

    /// Whether `handle_event` reads `EventRef::Raw`.
    ///
    /// Raw dispatch walks the whole ancestor path on every pointer move;
    /// knowing that nobody on it is listening turns that walk into an integer
    /// comparison. Raw events are widget-private — see the module docs on
    /// `event_system` — so the widgets that set this are the complete set of
    /// nodes raw dispatch has to visit.
    const READS_RAW_EVENTS: bool = false;

    fn get_key(&self) -> Option<&Key> {
        None
    }

    /// The style the element wrote.
    fn current_style(&self) -> &Style;

    /// [`Self::current_style`], for a templated element's changed style
    /// attributes to be written straight into a live widget. `None` for a
    /// widget no element can write a style into.
    fn style_mut(&mut self) -> Option<&mut Style> {
        None
    }

    /// Sets the key the element wrote, for a templated element whose key is
    /// computed per render. Ignored by a widget that takes no key.
    fn set_key(&mut self, _key: Option<Key>) {}

    /// The style under [`Self::current_style`].
    fn default_style(&self) -> Style {
        Style::new()
    }

    /// A hash of what the element wrote, interaction left out (the caller
    /// hashes that).
    fn props_hash(&self) -> u64;

    /// See [`Widgets::instantiate`].
    fn instantiate(&self) -> Self;

    /// Takes what `next` wrote and reports what has to be redone; [`sync`]
    /// does both for one field.
    fn update_from(&mut self, next: &Self) -> WidgetUpdateFlags;

    fn render(
        &self,
        node_id: xui_interface::NodeId,
        rect: Bounds,
        style: &ComputedStyle,
        writer: &mut RenderTreeWriter<'_>,
    );

    fn handle_event(
        &mut self,
        _event: EventRef<'_>,
        _cx: &mut crate::event_system::EventContext<'_>,
    ) -> EventResult {
        EventResult::Ignored
    }

    fn text_content(&self) -> Option<TextContent> {
        None
    }

    fn text_layout_props(&self, _style: &ComputedStyle) -> Option<TextProps> {
        None
    }

    /// The interaction properties and handlers, for a widget that takes them.
    fn interaction(&self) -> Option<(&InteractionProperties, &EventHandlers)> {
        None
    }

    /// [`Self::interaction`], mutably.
    fn interaction_mut(&mut self) -> Option<(&mut InteractionProperties, &mut EventHandlers)> {
        None
    }
}

/// Copies `next` into `current` when they differ, and reports `flags` if they
/// did — one field of a [`HostWidget::update_from`].
fn sync<T: PartialEq + Clone>(
    current: &mut T,
    next: &T,
    flags: WidgetUpdateFlags,
) -> WidgetUpdateFlags {
    if *current == *next {
        WidgetUpdateFlags::empty()
    } else {
        current.clone_from(next);
        flags
    }
}

fn widget_type<W: HostWidget>(_: &W) -> WidgetType {
    W::TYPE
}

fn reads_raw_events<W: HostWidget>(_: &W) -> bool {
    W::READS_RAW_EVENTS
}

/// Registers the host widgets: the [`Widgets`] enum, a `From` per widget, and
/// `with_widget!`, which every [`Widgets`] method dispatches through. What a
/// widget *does* is its [`HostWidget`] impl; nothing here needs to change when
/// that trait grows a method.
///
/// The leading `$` is passed in so the generated `with_widget!` can have
/// metavariables of its own.
macro_rules! define_widgets {
    ($d:tt $($name:ident => $widget:ty),+ $(,)?) => {
        #[derive(Debug)]
        pub enum Widgets {
            $(
                $name($widget),
            )+
        }

        $(
            impl From<$widget> for Widgets {
                fn from(widget: $widget) -> Self {
                    Self::$name(widget)
                }
            }
        )+

        /// `with_widget!(widgets, w => body)` evaluates `body` with `w` bound
        /// to the widget inside `widgets`, whichever variant it is.
        macro_rules! with_widget {
            ($d widgets:expr, $d w:ident => $d body:expr) => {
                match $d widgets {
                    $(
                        Widgets::$name($d w) => $d body,
                    )+
                }
            };
        }

        impl Widgets {
            pub fn update_from(&mut self, next: &Self) -> WidgetUpdateFlags {
                match (self, next) {
                    $(
                        (Self::$name(current), Self::$name(next)) => {
                            HostWidget::update_from(current, next)
                        }
                    )+
                    _ => WidgetUpdateFlags::TREE,
                }
            }
        }
    };
}

/// Implements [`HostWidget`] by forwarding to inherent methods of the same
/// names — how every widget wrote its part of the contract before the trait
/// existed. A widget leaves this list once it implements the trait itself.
macro_rules! legacy_host_widget {
    ($widget:ty => $ty:ident, interactive: $interactive:tt, fields: $fields:tt, raw: $raw:literal) => {
        impl HostWidget for $widget {
            const TYPE: WidgetType = WidgetType::$ty;
            const READS_RAW_EVENTS: bool = $raw;

            fn get_key(&self) -> Option<&Key> {
                <$widget>::get_key(self)
            }

            fn current_style(&self) -> &Style {
                <$widget>::current_style(self)
            }

            fn default_style(&self) -> Style {
                <$widget>::default_style(self)
            }

            fn props_hash(&self) -> u64 {
                <$widget>::props_hash(self)
            }

            fn instantiate(&self) -> Self {
                <$widget>::instantiate(self)
            }

            fn update_from(&mut self, next: &Self) -> WidgetUpdateFlags {
                <$widget>::update_from(self, next)
            }

            fn render(
                &self,
                node_id: xui_interface::NodeId,
                rect: Bounds,
                style: &ComputedStyle,
                writer: &mut RenderTreeWriter<'_>,
            ) {
                <$widget>::render(self, node_id, rect, style, writer)
            }

            fn handle_event(
                &mut self,
                event: EventRef<'_>,
                cx: &mut crate::event_system::EventContext<'_>,
            ) -> EventResult {
                <$widget>::handle_event(self, event, cx)
            }

            fn text_content(&self) -> Option<TextContent> {
                <$widget>::text_content(self)
            }

            fn text_layout_props(&self, style: &ComputedStyle) -> Option<TextProps> {
                <$widget>::text_layout_props(self, style)
            }

            legacy_host_widget!(@interaction $interactive);
            legacy_host_widget!(@fields $fields);
        }
    };
    // The `pub key` and `pub style` fields every element-built widget has.
    (@fields true) => {
        fn style_mut(&mut self) -> Option<&mut Style> {
            Some(&mut self.style)
        }

        fn set_key(&mut self, key: Option<Key>) {
            self.key = key;
        }
    };
    (@fields false) => {};
    (@interaction true) => {
        fn interaction(&self) -> Option<(&InteractionProperties, &EventHandlers)> {
            Some((&self.interaction, &self.event_handlers))
        }

        fn interaction_mut(&mut self) -> Option<(&mut InteractionProperties, &mut EventHandlers)> {
            Some((&mut self.interaction, &mut self.event_handlers))
        }
    };
    (@interaction false) => {};
}

define_widgets! {
    $
    Container => ContainerWidget,
    ZStack => ZStackWidget,
    Text => TextWidget,
    TextInput => TextInputWidget,
    Canvas => CanvasWidget,
    Image => ImageWidget,
    Icon => IconWidget,
    Grid => GridWidget,
    RootOverlayer => RootOverlayerWidget,
    Root => RootWidget,
}

legacy_host_widget!(ZStackWidget => ZStack, interactive: true, fields: true, raw: false);
legacy_host_widget!(TextInputWidget => TextInput, interactive: true, fields: true, raw: true);
legacy_host_widget!(CanvasWidget => Canvas, interactive: true, fields: true, raw: false);
legacy_host_widget!(ImageWidget => Image, interactive: true, fields: true, raw: false);
legacy_host_widget!(IconWidget => Icon, interactive: true, fields: true, raw: false);
legacy_host_widget!(GridWidget => Grid, interactive: true, fields: true, raw: false);
legacy_host_widget!(RootOverlayerWidget => RootOverlayer, interactive: false, fields: false, raw: false);

impl Widgets {
    pub fn event_handlers(&self) -> Option<&EventHandlers> {
        with_widget!(self, w => HostWidget::interaction(w)).map(|(_, handlers)| handlers)
    }

    pub fn take_event_handlers(&mut self) -> Option<EventHandlers> {
        with_widget!(self, w => HostWidget::interaction_mut(w))
            .map(|(_, handlers)| std::mem::take(handlers))
    }

    pub fn event_handlers_mut(&mut self) -> Option<&mut EventHandlers> {
        with_widget!(self, w => HostWidget::interaction_mut(w)).map(|(_, handlers)| handlers)
    }

    pub fn interaction_properties(&self) -> Option<&InteractionProperties> {
        with_widget!(self, w => HostWidget::interaction(w)).map(|(properties, _)| properties)
    }

    pub fn take_host_interaction(&mut self) -> Option<HostInteraction> {
        let (properties, handlers) = with_widget!(self, w => HostWidget::interaction_mut(w))?;
        let interaction = HostInteraction {
            properties: std::mem::take(properties),
            handlers: std::mem::take(handlers),
        };
        (!interaction.is_empty()).then_some(interaction)
    }

    /// [`Self::take_host_interaction`] for a description that may be shared,
    /// and so cannot be emptied.
    pub fn host_interaction(&self) -> Option<HostInteraction> {
        let (properties, handlers) = with_widget!(self, w => HostWidget::interaction(w))?;
        let interaction = HostInteraction {
            properties: properties.clone(),
            handlers: handlers.clone(),
        };
        (!interaction.is_empty()).then_some(interaction)
    }

    /// A live widget for a new host, built from this description: what the
    /// element wrote is copied, the widget's own state starts fresh, and
    /// handlers and interaction properties are left out because the
    /// interaction system owns them once committed.
    pub(crate) fn instantiate(&self) -> Self {
        with_widget!(self, w => HostWidget::instantiate(w).into())
    }

    pub fn node_type(&self) -> WidgetType {
        with_widget!(self, w => widget_type(w))
    }

    /// See [`HostWidget::READS_RAW_EVENTS`].
    pub(crate) fn reads_raw_events(&self) -> bool {
        with_widget!(self, w => reads_raw_events(w))
    }

    pub fn key(&self) -> Option<&Key> {
        with_widget!(self, w => HostWidget::get_key(w))
    }

    pub fn props_hash(&self) -> u64 {
        with_widget!(self, w => HostWidget::props_hash(w))
    }

    pub fn default_style(&self) -> Style {
        with_widget!(self, w => HostWidget::default_style(w))
    }

    pub fn style(&self) -> &Style {
        with_widget!(self, w => HostWidget::current_style(w))
    }

    pub fn style_scope(&self) -> Option<&Style> {
        None
    }

    pub(crate) fn style_mut(&mut self) -> Option<&mut Style> {
        with_widget!(self, w => HostWidget::style_mut(w))
    }

    pub(crate) fn set_key(&mut self, key: Option<Key>) {
        with_widget!(self, w => HostWidget::set_key(w, key))
    }

    /// Gives a widget built from a description the interaction it would have
    /// had as written: [`Self::instantiate`] leaves interaction out.
    pub(crate) fn set_interaction(&mut self, interaction: HostInteraction) {
        if let Some((properties, handlers)) = with_widget!(self, w => HostWidget::interaction_mut(w)) {
            *properties = interaction.properties;
            *handlers = interaction.handlers;
        }
    }

    pub(crate) fn render(
        &self,
        node_id: xui_interface::NodeId,
        rect: Bounds,
        style: &ComputedStyle,
        writer: &mut RenderTreeWriter<'_>,
    ) {
        with_widget!(self, w => HostWidget::render(w, node_id, rect, style, writer))
    }

    pub fn handle_event(
        &mut self,
        event: EventRef<'_>,
        cx: &mut crate::event_system::EventContext<'_>,
    ) -> EventResult {
        with_widget!(self, w => HostWidget::handle_event(w, event, cx))
    }

    pub fn on_click(&mut self) {}

    pub fn text(&self) -> Option<TextContent> {
        with_widget!(self, w => HostWidget::text_content(w))
    }

    pub fn text_layout_props(&self, style: &ComputedStyle) -> Option<TextProps> {
        with_widget!(self, w => HostWidget::text_layout_props(w, style))
    }

    pub(crate) fn platform_text_input_session(
        &self,
        content_rect: Bounds,
        text_layout: &dyn crate::text::TextLayoutQuery,
    ) -> Option<xui_interface::TextInputSession> {
        match self {
            Self::TextInput(widget) => {
                Some(widget.platform_text_input_session(content_rect, text_layout))
            }
            _ => None,
        }
    }
}

/// What an element describes: a widget as the DSL built it.
///
/// Immutable, so one description can be held by a memo, reused across renders
/// and mounted at several places at once. A host never runs on it: mounting
/// makes a [`WidgetI`] from it with [`WidgetProto::instantiate`], and later
/// renders update that instance from the next description. Keeping the two
/// apart is what lets the reconciler skip a description it has already
/// committed by pointer alone.
#[derive(Debug, Clone)]
pub struct WidgetProto {
    widget: Rc<Widgets>,
}

impl WidgetProto {
    pub fn new(widget: impl Into<Widgets>) -> Self {
        Self {
            widget: Rc::new(widget.into()),
        }
    }

    pub(crate) fn widgets(&self) -> &Widgets {
        &self.widget
    }

    /// Whether `self` and `other` are the same description, not merely equal
    /// ones.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.widget, &other.widget)
    }

    pub fn key(&self) -> Option<Key> {
        self.widget.key().cloned()
    }

    pub fn node_type(&self) -> WidgetType {
        self.widget.node_type()
    }

    pub fn props_hash(&self) -> u64 {
        props_hash(&(
            self.widget.props_hash(),
            self.widget.interaction_properties(),
        ))
    }

    pub fn text(&self) -> Option<TextContent> {
        self.widget.text()
    }

    /// The handlers and interaction properties to commit with this
    /// description. Moved out when nothing else holds the description, cloned
    /// (handlers are `Rc`s) when something does.
    pub(crate) fn take_host_interaction(&mut self) -> Option<HostInteraction> {
        match Rc::get_mut(&mut self.widget) {
            Some(widget) => widget.take_host_interaction(),
            None => self.widget.host_interaction(),
        }
    }

    /// A live widget for a new host. See [`Widgets::instantiate`].
    pub(crate) fn instantiate(&self) -> WidgetI {
        WidgetI::new(self.widget.instantiate())
    }
}

#[derive(Debug, Clone)]
pub struct WidgetI {
    widget: Rc<RefCell<Widgets>>,
}

impl WidgetI {
    pub fn new(widget: impl Into<Widgets>) -> Self {
        Self {
            widget: Rc::new(RefCell::new(widget.into())),
        }
    }

    pub(crate) fn with_widgets<R>(&self, f: impl FnOnce(&Widgets) -> R) -> R {
        let widget = self.widget.borrow();
        f(&widget)
    }

    pub(crate) fn with_widgets_mut<R>(&self, f: impl FnOnce(&mut Widgets) -> R) -> R {
        let mut widget = self.widget.borrow_mut();
        f(&mut widget)
    }

    pub fn key(&self) -> Option<Key> {
        self.with_widgets(|widget| widget.key().cloned())
    }

    pub fn node_type(&self) -> WidgetType {
        self.with_widgets(|widget| widget.node_type())
    }

    pub(crate) fn reads_raw_events(&self) -> bool {
        self.with_widgets(|widget| widget.reads_raw_events())
    }

    pub fn props_hash(&self) -> u64 {
        self.with_widgets(|widget| {
            props_hash(&(widget.props_hash(), widget.interaction_properties()))
        })
    }

    pub fn text(&self) -> Option<TextContent> {
        self.with_widgets(|widget| widget.text())
    }

    pub(crate) fn platform_text_input_session(
        &self,
        content_rect: Bounds,
        text_layout: &dyn crate::text::TextLayoutQuery,
    ) -> Option<xui_interface::TextInputSession> {
        self.with_widgets(|widget| widget.platform_text_input_session(content_rect, text_layout))
    }

    pub fn intrinsic_size(&self) -> Option<Size<f32>> {
        self.with_widgets(|widget| match widget {
            Widgets::Image(image) => image.intrinsic_size(),
            _ => None,
        })
    }

    pub(crate) fn transition(&self) -> Option<xui_interface::Transition> {
        self.with_widgets(|widget| widget.style().transition_config())
    }

    pub(crate) fn animation(&self) -> Option<xui_interface::Animation> {
        self.with_widgets(|widget| widget.style().animation_config().cloned())
    }

    #[inline]
    pub fn take_event_handlers(&self) -> Option<EventHandlers> {
        self.with_widgets_mut(|w| w.take_event_handlers())
    }

    #[inline]
    pub(crate) fn take_host_interaction(&self) -> Option<HostInteraction> {
        self.with_widgets_mut(|w| w.take_host_interaction())
    }

    #[inline]
    pub(crate) fn render(
        &self,
        node_id: xui_interface::NodeId,
        rect: Bounds,
        style: &ComputedStyle,
        writer: &mut RenderTreeWriter<'_>,
    ) {
        self.with_widgets(|widget| widget.render(node_id, rect, style, writer));
    }

    pub fn handle_event(
        &self,
        event: xui_interface::EventRef<'_>,
        cx: &mut crate::event_system::EventContext<'_>,
    ) -> xui_interface::EventResult {
        self.with_widgets_mut(|widget| widget.handle_event(event, cx))
    }

    pub fn on_click(&self) {
        self.with_widgets_mut(|widget| widget.on_click());
    }

    pub fn update_from(&self, next: &Self) -> xui_interface::WidgetUpdateFlags {
        next.with_widgets(|next| self.update_from_widgets(next))
    }

    pub(crate) fn update_from_widgets(&self, next: &Widgets) -> xui_interface::WidgetUpdateFlags {
        self.with_widgets_mut(|current| current.update_from(next))
    }

    pub fn computed_style(
        &self,
        parent: &ComputedStyle,
        theme: &crate::style::Theme,
    ) -> ComputedStyle {
        crate::layout::computed_style_for_widget(
            self,
            parent,
            theme,
            xui_interface::WidgetState::empty(),
        )
    }
}

pub(crate) fn widget_element_desc(
    widget: impl Into<Widgets>,
    children: Vec<ElementDesc>,
) -> ElementDesc {
    WidgetDesc::new(WidgetProto::new(widget), children).into()
}

pub trait WithChildren {
    fn with_children(self, children: Vec<ElementDesc>) -> Self;
}

impl ComponentDesc {
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn props<P>(mut self, props: P) -> Self
    where
        P: Any,
    {
        self.props = Some(Rc::new(props));
        self
    }
}

// pub fn label(text: impl Into<TextContent>) -> LabelWidget {
//     LabelWidget::new(text)
// }

/// `<text>` — starts empty; content comes from `text={..}` or `<text>{..}</text>`.
/// Hand-written call sites that already have the content want
/// [`TextWidget::new`] instead.
pub fn text() -> TextWidget {
    TextWidget::new("")
}

#[doc(hidden)]
pub fn text_input() -> TextInputWidget {
    TextInputWidget::new()
}

// pub fn column() -> ColumnWidget {
//     ColumnWidget::new()
// }

// pub fn row() -> RowWidget {
//     RowWidget::new()
// }

pub fn container() -> ContainerWidget {
    ContainerWidget::new()
}

/// `<row>` — a [`container`] preset, not a distinct widget.
///
/// Direction is the only thing a row adds over a container, so it is a
/// constructor rather than a component. Being a host tag is what lets it take
/// the full style vocabulary *and* event handlers:
/// `<row gap={8.0} on_click={handler}>`. A component would have to redeclare
/// every one of those as a prop, and could not accept the handlers at all.
pub fn row() -> ContainerWidget {
    ContainerWidget::new().flex_direction(FlexDirectionStyle::Row)
}

/// `<column>` — a [`container`] preset. See [`row`].
pub fn column() -> ContainerWidget {
    ContainerWidget::new().flex_direction(FlexDirectionStyle::Column)
}

/// `<center>` — a [`container`] preset, not a distinct widget. See [`row`].
///
/// Fills its parent in both axes by default. Centering distributes the free
/// space inside the container, and a content-sized container has none, so a
/// hugging `<center>` would look as if it centered nothing. A `width`,
/// `height` or `size` given to it still overrides the fill.
pub fn center() -> ContainerWidget {
    ContainerWidget::new()
        .flex_direction(FlexDirectionStyle::Column)
        .style(Style::from_patch(
            xui_interface::StylePatch::default()
                .width(xui_interface::core::Sizing::Fill)
                .height(xui_interface::core::Sizing::Fill)
                .justify(xui_interface::JustifyStyle::Center)
                .align(xui_interface::AlignStyle::Center),
        ))
}

pub fn z_stack() -> ZStackWidget {
    ZStackWidget::new()
}

pub fn grid() -> GridWidget {
    GridWidget::new()
}

/// `<canvas>` — starts with a detached controller that `controller={..}`
/// replaces. Every tag constructor takes no arguments so that the `xui!` macro
/// needs no per-tag knowledge.
pub fn canvas() -> CanvasWidget {
    CanvasWidget::new(CanvasController::new())
}

pub fn image() -> ImageWidget {
    ImageWidget::new()
}

pub fn icon() -> IconWidget {
    IconWidget::new()
}

pub(crate) fn root_overlayer_widget() -> RootOverlayerWidget {
    RootOverlayerWidget::new()
}

// pub fn style_scope(style: Style) -> StyleScopeWidget {
//     StyleScopeWidget::new(style)
// }

// pub fn scroll_scope() -> ScrollScope {
//     ScrollScope::new()
// }

pub fn root_widget() -> WidgetI {
    WidgetI::new(RootWidget::default())
}

pub fn component(render: ComponentRender) -> ComponentDesc {
    ComponentDesc::new(render, None)
}

pub(super) fn props_hash<T: Hash>(props: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    props.hash(&mut hasher);
    hasher.finish()
}
