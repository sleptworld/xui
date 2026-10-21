use std::any::Any;
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::rc::Rc;

pub use crate::event_system::callbacks::EventHandlers;
use crate::event_system::interaction::{HostInteraction, InteractionProperties};
use xui_interface::core::Bounds;
use xui_interface::style::FlexDirectionStyle;
use xui_interface::{EventRef, EventResult, TextContent, TextProps, WidgetUpdateFlags};
pub use xui_interface::{Style, WidgetType};

pub(crate) mod host_style;
pub(crate) mod roles;

pub(crate) use host_style::HostStyle;

use crate::core::Size;
use crate::element::{ComponentDesc, ElementDesc, WidgetDesc};
use crate::fiber::{ComponentRender, Key};
use crate::render::RenderTreeWriter;
use crate::state::HookContext;
use crate::style::ComputedStyle;

pub struct WidgetInstance<W: WidgetT> {
    props: W::Props,
    state: W::State,
}

pub trait WidgetT {
    type Props: 'static;
    type State: 'static;

    fn create_state(props: &Self::Props) -> Self::State;
    fn update(old_props: &Self::Props, new_props: &Self::Props, state: &mut Self::State) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WidgetBox<W: WidgetT> {
    _marker: PhantomData<W>,
}

macro_rules! no_event_handler_methods {
    () => {
        pub fn event_handlers(&self) -> Option<&crate::event_system::callbacks::EventHandlers> {
            None
        }

        pub fn event_handlers_mut(
            &mut self,
        ) -> Option<&mut crate::event_system::callbacks::EventHandlers> {
            None
        }

        pub fn take_event_handlers(
            &mut self,
        ) -> Option<crate::event_system::callbacks::EventHandlers> {
            None
        }

        pub fn interaction_properties(
            &self,
        ) -> Option<&crate::event_system::interaction::InteractionProperties> {
            None
        }

        pub fn take_host_interaction(
            &mut self,
        ) -> Option<crate::event_system::interaction::HostInteraction> {
            None
        }
    };
}

macro_rules! event_handler_methods {
    () => {
        pub fn event_handlers(&self) -> Option<&EventHandlers> {
            Some(&self.event_handlers)
        }

        pub fn event_handlers_mut(&mut self) -> Option<&mut EventHandlers> {
            Some(&mut self.event_handlers)
        }

        pub fn take_event_handlers(&mut self) -> Option<EventHandlers> {
            Some(std::mem::replace(
                &mut self.event_handlers,
                EventHandlers::default(),
            ))
        }

        pub fn interaction_properties(
            &self,
        ) -> Option<&crate::event_system::interaction::InteractionProperties> {
            Some(&self.interaction)
        }

        pub fn take_host_interaction(
            &mut self,
        ) -> Option<crate::event_system::interaction::HostInteraction> {
            let interaction = crate::event_system::interaction::HostInteraction {
                properties: std::mem::take(&mut self.interaction),
                handlers: std::mem::take(&mut self.event_handlers),
            };
            (!interaction.is_empty()).then_some(interaction)
        }

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

macro_rules! define_widgets {
    (@role paint) => {
        fn as_paint(&self) -> Option<&dyn roles::PaintContent> {
            Some(self)
        }
    };
    (@role text) => {
        fn as_text(&self) -> Option<&dyn roles::TextSource> {
            Some(self)
        }
    };
    (@role events) => {
        fn as_handle_event(&self) -> Option<&dyn roles::HandleEvent> {
            Some(self)
        }

        fn as_handle_event_mut(&mut self) -> Option<&mut dyn roles::HandleEvent> {
            Some(self)
        }
    };
    (@role text_input) => {
        fn as_text_input(&self) -> Option<&dyn roles::PlatformTextInput> {
            Some(self)
        }
    };
    (@role intrinsic_size) => {
        fn as_intrinsic_size(&self) -> Option<&dyn roles::IntrinsicSize> {
            Some(self)
        }
    };
    (@role arrange) => {
        fn as_arrange(&self) -> Option<&dyn roles::ArrangeChildren> {
            Some(self)
        }
    };

    ($($name:ident => $widget:ty [$($role:ident),* $(,)?]),+ $(,)?) => {
        $(
            impl roles::WidgetRoles for $widget {
                $(define_widgets!(@role $role);)*
            }
        )+

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

        impl Widgets {
            pub fn event_handlers(&self) -> Option<&EventHandlers> {
                match self {
                    $(
                        Self::$name(widget) => widget.event_handlers(),
                    )+
                }
            }

            pub fn take_event_handlers(&mut self) -> Option<EventHandlers> {

                match self {
                    $(
                        Self::$name(widget) => widget.take_event_handlers(),
                    )+
                }

            }

            pub fn event_handlers_mut(&mut self) -> Option<&mut EventHandlers> {
                match self {
                    $(
                        Self::$name(widget) => widget.event_handlers_mut(),
                    )+
                }
            }

            pub fn interaction_properties(&self) -> Option<&InteractionProperties> {
                match self {
                    $(
                        Self::$name(widget) => widget.interaction_properties(),
                    )+
                }
            }

            pub fn take_host_interaction(&mut self) -> Option<HostInteraction> {
                match self {
                    $(
                        Self::$name(widget) => widget.take_host_interaction(),
                    )+
                }
            }

            pub fn node_type(&self) -> WidgetType {
                match self {
                    $(
                        Self::$name(widget) => widget.node_type(),
                    )+
                }
            }

            /// Whether this widget reads `EventRef::Raw` in `handle_event`.
            ///
            /// Raw dispatch walks the whole ancestor path on every pointer
            /// move; knowing that nobody on it is listening turns that walk
            /// into an integer comparison. Answered by the widget's own
            /// [`roles::HandleEvent`], so it cannot drift from reality.
            pub(crate) fn reads_raw_events(&self) -> bool {
                self.roles().as_handle_event().is_some_and(|it| it.raw_events())
            }

            /// The widget's declared roles.
            pub(crate) fn roles(&self) -> &dyn roles::WidgetRoles {
                match self {
                    $(Self::$name(widget) => widget,)+
                }
            }

            pub(crate) fn roles_mut(&mut self) -> &mut dyn roles::WidgetRoles {
                match self {
                    $(Self::$name(widget) => widget,)+
                }
            }

            pub fn key(&self) -> Option<&Key> {
                match self {
                    $(
                        Self::$name(widget) => widget.get_key(),
                    )+
                }
            }

            pub fn props_hash(&self) -> u64 {
                match self {
                    $(
                        Self::$name(widget) => widget.props_hash(),
                    )+
                }
            }

            pub fn update_from(&mut self, next: &Self) -> WidgetUpdateFlags {
                match (self, next) {
                    $(
                        (Self::$name(current), Self::$name(next)) => current.update_from(next),
                    )+
                    _ => WidgetUpdateFlags::TREE,
                }
            }

            pub fn default_style(&self) -> Style {
                match self {
                    $(
                        Self::$name(widget) => widget.default_style(),
                    )+
                }
            }

            pub fn style(&self) -> &Style {
                match self {
                    $(
                        Self::$name(widget) => widget.current_style(),
                    )+
                }
            }

            pub fn style_scope(&self) -> Option<&Style> {
                None
            }

            /// Every widget still answers from its `Style`; a widget moved
            /// onto a node definition answers through
            /// [`host_style::NodeStyle`] instead.
            pub(crate) fn with_host_style<R>(&self, f: impl FnOnce(&dyn HostStyle) -> R) -> R {
                f(&host_style::LegacyStyle::new(self))
            }

            /// Paints the widget's own content, if it has any.
            pub(crate) fn render(
                &self,
                node_id: xui_interface::NodeId,
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

            pub fn on_click(&mut self) {}

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
        }
    };
}

// Each widget's roles (see `roles`): the behaviour its own code implements.
// A role listed here that the widget does not implement fails to compile.
define_widgets! {
    Container => ContainerWidget [arrange],
    ZStack => ZStackWidget [arrange],
    Text => TextWidget [paint, text],
    TextInput => TextInputWidget [paint, text, events, text_input],
    Canvas => CanvasWidget [paint],
    Image => ImageWidget [paint, intrinsic_size],
    Icon => IconWidget [paint],
    Grid => GridWidget [arrange],
    RootOverlayer => RootOverlayerWidget [],
    Root => RootWidget [arrange],
}

impl Widgets {
    /// Whether the runtime paints this widget's decoration — background,
    /// border, corner radius, shadow — under its content.
    ///
    /// A node on a node definition decorates when it selects a decoration
    /// capability (`CapMask::DECORATION`); until every widget has one, this
    /// lists the widgets that used to draw it themselves.
    pub(crate) fn decorates(&self) -> bool {
        matches!(
            self,
            Widgets::Container(_) | Widgets::ZStack(_) | Widgets::Image(_) | Widgets::Root(_)
        )
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
        self.with_widgets(|widget| {
            widget
                .roles()
                .as_intrinsic_size()
                .and_then(|it| it.intrinsic_size())
        })
    }

    /// Whether the runtime paints this widget's decoration under its content.
    pub(crate) fn decorates(&self) -> bool {
        self.with_widgets(Widgets::decorates)
    }

    /// Runs `f` on what this widget answers the runtime about its style.
    pub(crate) fn with_host_style<R>(&self, f: impl FnOnce(&dyn HostStyle) -> R) -> R {
        self.with_widgets(|widget| widget.with_host_style(f))
    }

    pub(crate) fn transition(&self) -> Option<xui_interface::Transition> {
        self.with_host_style(|style| style.transition())
    }

    pub(crate) fn animation(&self) -> Option<xui_interface::Animation> {
        self.with_host_style(|style| style.animation())
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
        self.with_widgets_mut(|current| next.with_widgets(|next| current.update_from(next)))
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
    WidgetDesc::new(WidgetI::new(widget), children).into()
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
