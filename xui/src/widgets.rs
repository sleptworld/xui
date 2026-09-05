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
use xui_macros::style;

mod utils;

use crate::core::Size;
use crate::element::{ComponentDesc, ElementDesc, WidgetDesc};
use crate::fiber::{ComponentRender, Key};
use crate::render::RenderTreeWriter;
use crate::state::HookContext;
use crate::style::ComputedStyle;

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
pub(crate) use canvas::canvas_text_slot;
pub use canvas::{
    CanvasContent, CanvasController, CanvasPainter, CanvasPick, CanvasPickTag, CanvasTextLayout,
    CanvasTextMetrics, CanvasWidget,
};
pub use container::ContainerWidget;
pub use grid::*;
pub use icon::{IconData, IconLayer, IconStroke, IconWidget, SvgIconError};
pub use image::ImageWidget;
pub(crate) use overlay::RootOverlayerWidget;
pub use overlay::{
    OverlayChild, OverlayEntry, OverlayEntryId, OverlayEntryOptions, OverlayModelError,
    OverlayScope, OverlayScopeId,
};
pub use root::RootWidget;
pub use text::TextWidget;
pub use text_input::TextInputWidget;
#[doc(hidden)]
pub use text_input::keymap::{TextCommand, TextKeymap};
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
    ($($name:ident => $widget:ty $([$raw:ident])?),+ $(,)?) => {
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
            /// into an integer comparison. Declared in the widget table rather
            /// than in a hand-kept `match`, so it cannot drift from reality.
            pub(crate) fn reads_raw_events(&self) -> bool {
                match self {
                    $(
                        Self::$name(_) => false $( || stringify!($raw) == "raw" )? ,
                    )+
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

            pub(crate) fn render(
                &self,
                node_id: xui_interface::NodeId,
                rect: Bounds,
                style: &ComputedStyle,
                writer: &mut RenderTreeWriter<'_>,
            ) {
                match self {
                    $(
                        Self::$name(widget) => widget.render(node_id, rect, style, writer),
                    )+
                }
            }

            pub fn handle_event(
                &mut self,
                event: EventRef<'_>,
                cx: &mut crate::event_system::EventContext<'_>,
            ) -> EventResult {
                match self {
                    $(
                        Self::$name(widget) => widget.handle_event(event, cx),
                    )+
                }
            }

            pub fn on_click(&mut self) {}

            pub fn text(&self) -> Option<TextContent> {
                match self {
                    $(
                        Self::$name(widget) => widget.text_content(),
                    )+
                }
            }

            pub fn text_layout_props(&self, style: &ComputedStyle) -> Option<TextProps> {
                match self {
                    $(
                        Self::$name(widget) => widget.text_layout_props(style),
                    )+
                }
            }

            pub(crate) fn platform_text_input_session(
                &self,
                node_rect: Bounds,
                text_layout: &dyn crate::text::TextLayoutQuery,
            ) -> Option<xui_interface::TextInputSession> {
                match self {
                    Self::TextInput(widget) => {
                        Some(widget.platform_text_input_session(node_rect, text_layout))
                    }
                    _ => None,
                }
            }
        }
    };
}

// `[raw]` marks a widget that reads `EventRef::Raw` in its `handle_event`.
// Raw events are widget-private — see the module docs on `event_system` — so
// this list is the complete set of nodes raw dispatch has to visit.
define_widgets! {
    Container => ContainerWidget,
    ZStack => ZStackWidget,
    Text => TextWidget,
    TextInput => TextInputWidget [raw],
    Canvas => CanvasWidget,
    Image => ImageWidget,
    Icon => IconWidget,
    Grid => GridWidget,
    RootOverlayer => RootOverlayerWidget,
    Root => RootWidget,
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
        node_rect: Bounds,
        text_layout: &dyn crate::text::TextLayoutQuery,
    ) -> Option<xui_interface::TextInputSession> {
        self.with_widgets(|widget| widget.platform_text_input_session(node_rect, text_layout))
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
pub fn center() -> ContainerWidget {
    ContainerWidget::new()
        .flex_direction(FlexDirectionStyle::Column)
        .style(style! {
            justify: xui_interface::JustifyStyle::Center,
            align: xui_interface::AlignStyle::Center
        })
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
