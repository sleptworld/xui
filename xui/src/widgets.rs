use std::any::Any;
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

pub use crate::event_system::callbacks::EventHandlers;
use xui_animation::Transition;
use xui_interface::{EventRef, EventResult, TextContent, TextProps, WidgetUpdateFlags};
pub use xui_interface::{Style, WidgetType};

use crate::core::{Rect, Size};
use crate::element::{ComponentDesc, ElementDesc, WidgetDesc};
use crate::fiber::{ComponentRender, Key};
use crate::render::RenderTreeWriter;
use crate::state::HookContext;
use crate::style::ComputedStyle;

macro_rules! event_handler_methods {
    () => {
        pub fn shortcut(
            mut self,
            shortcut: xui_interface::Shortcut,
            command: xui_interface::CommandId,
        ) -> Self {
            if let Some(binding) = self
                .event_handlers
                .shortcuts
                .iter_mut()
                .find(|binding| binding.shortcut == shortcut)
            {
                binding.command = command;
            } else {
                self.event_handlers
                    .shortcuts
                    .push(xui_interface::ShortcutBinding { shortcut, command });
            }
            self
        }

        pub fn on_command(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::CommandEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_command = Some(Box::new(handler));
            self
        }

        pub fn on_event(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::SemanticEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_event = Some(Box::new(handler));
            self
        }

        pub fn on_event_capture(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::SemanticEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_event_capture = Some(Box::new(handler));
            self
        }

        pub fn on_click(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::ClickEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_click = Some(Box::new(handler));
            self
        }

        pub fn on_click_capture(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::ClickEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_click_capture = Some(Box::new(handler));
            self
        }

        pub fn on_double_click(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::ClickEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_double_click = Some(Box::new(handler));
            self
        }

        pub fn on_double_click_capture(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::ClickEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_double_click_capture = Some(Box::new(handler));
            self
        }

        pub fn on_context_menu(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::ContextMenuEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_context_menu = Some(Box::new(handler));
            self
        }

        pub fn on_context_menu_capture(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::ContextMenuEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_context_menu_capture = Some(Box::new(handler));
            self
        }

        pub fn on_hover_enter(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::HoverEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_hover_enter = Some(Box::new(handler));
            self
        }

        pub fn on_hover_leave(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::HoverEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_hover_leave = Some(Box::new(handler));
            self
        }

        pub fn on_hover_change(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::HoverChangeEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_hover_change = Some(Box::new(handler));
            self
        }

        pub fn on_press_start(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::PressEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_press_start = Some(Box::new(handler));
            self
        }

        pub fn on_press_start_capture(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::PressEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_press_start_capture = Some(Box::new(handler));
            self
        }

        pub fn on_press_end(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::PressEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_press_end = Some(Box::new(handler));
            self
        }

        pub fn on_press_end_capture(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::PressEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_press_end_capture = Some(Box::new(handler));
            self
        }

        pub fn on_press_cancel(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::PressEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_press_cancel = Some(Box::new(handler));
            self
        }

        pub fn on_press_cancel_capture(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::PressEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_press_cancel_capture = Some(Box::new(handler));
            self
        }

        pub fn on_focus(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::FocusEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_focus = Some(Box::new(handler));
            self
        }

        pub fn on_blur(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::FocusEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_blur = Some(Box::new(handler));
            self
        }

        pub fn on_focus_in(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::FocusEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_focus_in = Some(Box::new(handler));
            self
        }

        pub fn on_focus_in_capture(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::FocusEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_focus_in_capture = Some(Box::new(handler));
            self
        }

        pub fn on_focus_out(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::FocusEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_focus_out = Some(Box::new(handler));
            self
        }

        pub fn on_focus_out_capture(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::FocusEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_focus_out_capture = Some(Box::new(handler));
            self
        }

        pub fn on_drag_start(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::DragEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_drag_start = Some(Box::new(handler));
            self
        }

        pub fn on_drag_start_capture(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::DragEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_drag_start_capture = Some(Box::new(handler));
            self
        }

        pub fn on_drag_move(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::DragEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_drag_move = Some(Box::new(handler));
            self
        }

        pub fn on_drag_move_capture(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::DragEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_drag_move_capture = Some(Box::new(handler));
            self
        }

        pub fn on_drag_end(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::DragEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_drag_end = Some(Box::new(handler));
            self
        }

        pub fn on_drag_end_capture(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::DragEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_drag_end_capture = Some(Box::new(handler));
            self
        }

        pub fn on_drag_cancel(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::DragEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_drag_cancel = Some(Box::new(handler));
            self
        }

        pub fn on_drag_cancel_capture(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::DragEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_drag_cancel_capture = Some(Box::new(handler));
            self
        }

        pub fn on_scroll(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::ScrollEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_scroll = Some(Box::new(handler));
            self
        }

        pub fn on_scroll_capture(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::ScrollEvent,
                &mut crate::event_system::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_scroll_capture = Some(Box::new(handler));
            self
        }
    };
}

mod container;
mod icon;
mod image;
mod root;
mod text;
mod text_input;
mod z_stack;

pub use container::ContainerWidget;
pub use icon::{IconData, IconLayer, IconStroke, IconWidget, SvgIconError};
pub use image::ImageWidget;
pub use root::RootWidget;
pub use text::TextWidget;
#[doc(hidden)]
pub use text_input::TextInputWidget;
pub use text_input::keymap::{TextCommand, TextKeymap};
pub use text_input::{TextController, TextInputChange};
pub use z_stack::ZStackWidget;

pub type RootComponentRender = fn(&mut HookContext) -> ElementDesc;

macro_rules! define_widgets {
    ($($name:ident => $widget:ty),+ $(,)?) => {
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
            pub fn event_handlers_mut(&mut self) -> &mut EventHandlers {
                match self {
                    $(
                        Self::$name(widget) => &mut widget.event_handlers,
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
                rect: Rect,
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
                node_rect: Rect,
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

define_widgets! {
    Container => ContainerWidget,
    ZStack => ZStackWidget,
    Text => TextWidget,
    TextInput => TextInputWidget,
    Image => ImageWidget,
    Icon => IconWidget,
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

    pub fn props_hash(&self) -> u64 {
        self.with_widgets(|widget| widget.props_hash())
    }

    pub fn text(&self) -> Option<TextContent> {
        self.with_widgets(|widget| widget.text())
    }

    pub(crate) fn platform_text_input_session(
        &self,
        node_rect: Rect,
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

    pub(crate) fn transition(&self) -> Option<Transition> {
        self.with_widgets(|w| match w {
            Widgets::Container(c) => c.transition,
            Widgets::ZStack(stack) => stack.transition,
            _ => None,
        })
    }

    pub fn take_event_handlers(&self) -> EventHandlers {
        self.with_widgets_mut(|widget| std::mem::take(widget.event_handlers_mut()))
    }

    pub(crate) fn render(
        &self,
        node_id: xui_interface::NodeId,
        rect: Rect,
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
        P: Any + Hash,
    {
        self.props_hash = props_hash(&props);
        self.props = Some(Rc::new(props));
        self
    }

    pub fn props_with_hash<P>(mut self, props: P, props_hash: u64) -> Self
    where
        P: Any,
    {
        self.props_hash = props_hash;
        self.props = Some(Rc::new(props));
        self
    }
}

// pub fn label(text: impl Into<TextContent>) -> LabelWidget {
//     LabelWidget::new(text)
// }

pub fn text(text: impl Into<xui_interface::TextContent>) -> TextWidget {
    TextWidget::new(text)
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

pub fn z_stack() -> ZStackWidget {
    ZStackWidget::new()
}

pub fn image() -> ImageWidget {
    ImageWidget::new()
}

pub fn icon(data: IconData) -> IconWidget {
    IconWidget::new(data)
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
    ComponentDesc::new(render, None, 0, Vec::new())
}

pub(super) fn props_hash<T: Hash>(props: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    props.hash(&mut hasher);
    hasher.finish()
}
