use std::any::Any;
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

pub use crate::event_system::callbacks::EventHandlers;
use xui_interface::TextContent;
pub use xui_interface::{Widget, WidgetType};

use crate::animation::{StyleAnimation, StyleAnimationRule};
use crate::core::Rect;
use crate::element::{ComponentDesc, ElementDesc, WidgetDesc};
use crate::fiber::{ComponentRender, Key};
use crate::render::PaintCommand;
use crate::state::HookContext;
use crate::style::{ComputedStyle, Style};

macro_rules! event_handler_methods {
    () => {
        pub fn on_event(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::events::SemanticEvent,
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
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
                &mut xui_interface::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_scroll_capture = Some(Box::new(handler));
            self
        }
    };
}

macro_rules! animated_style_methods {
    ($field:ident) => {
        pub fn animated_style(mut self, animated_style: crate::animation::AnimatedStyle) -> Self {
            self.$field = animated_style;
            self
        }

        pub fn animation(
            mut self,
            trigger: xui_interface::EventTrigger,
            style: crate::animation::AnimableStyle,
            transition: crate::animation::AnimationTransition,
        ) -> Self {
            self.$field
                .animations
                .push(crate::animation::StyleAnimation::new(
                    trigger, style, transition,
                ));
            self
        }

        pub(crate) fn style_animations(&self) -> &[crate::animation::StyleAnimation] {
            &self.$field.animations
        }
    };
}

mod button;
mod column;
mod container;
mod label;
mod root;
mod row;
mod scroll_scope;
mod style_scope;
mod text;

pub use button::ButtonWidget;
pub use column::ColumnWidget;
pub use container::ContainerWidget;
pub use label::LabelWidget;
pub use root::RootWidget;
pub use row::RowWidget;
pub use scroll_scope::ScrollScope;
pub use style_scope::StyleScopeWidget;
pub use text::TextWidget;

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

            pub(crate) fn style_animations(&self) -> &[StyleAnimation] {
                match self {
                    $(
                        Self::$name(widget) => widget.style_animations(),
                    )+
                }
            }
        }

        impl Widget for Widgets {
            fn node_type(&self) -> WidgetType {
                match self {
                    $(
                        Self::$name(widget) => widget.node_type(),
                    )+
                }
            }

            fn key(&self) -> Option<&Key> {
                match self {
                    $(
                        Self::$name(widget) => widget.key(),
                    )+
                }
            }

            fn props_hash(&self) -> u64 {
                match self {
                    $(
                        Self::$name(widget) => widget.props_hash(),
                    )+
                }
            }

            fn update_from(&mut self, next: &Self) -> xui_interface::DirtyFlags {
                match (self, next) {
                    $(
                        (Self::$name(current), Self::$name(next)) => current.update_from(next),
                    )+
                    _ => {
                        xui_interface::DirtyFlags::TREE
                            | xui_interface::DirtyFlags::LAYOUT
                            | xui_interface::DirtyFlags::PAINT
                    }
                }
            }

            fn default_style(&self) -> Style {
                match self {
                    $(
                        Self::$name(widget) => widget.default_style(),
                    )+
                }
            }

            fn style(&self) -> &Style {
                match self {
                    $(
                        Self::$name(widget) => widget.style(),
                    )+
                }
            }

            fn state_style(&self, state: xui_interface::WidgetState) -> Style {
                match self {
                    $(
                        Self::$name(widget) => widget.state_style(state),
                    )+
                }
            }

            fn state(&self) -> xui_interface::WidgetState {
                match self {
                    $(
                        Self::$name(widget) => widget.state(),
                    )+
                }
            }

            fn style_scope(&self) -> Option<&Style> {
                match self {
                    $(
                        Self::$name(widget) => widget.style_scope(),
                    )+
                }
            }

            fn paint(
                &self,
                rect: xui_interface::Rect,
                style: &ComputedStyle,
                commands: &mut Vec<xui_interface::PaintCommand>,
            ) {
                match self {
                    $(
                        Self::$name(widget) => widget.paint(rect, style, commands),
                    )+
                }
            }

            fn handle_event(
                &mut self,
                event: &xui_interface::Event,
                cx: &mut xui_interface::EventContext<'_>,
            ) -> xui_interface::EventResult {
                match self {
                    $(
                        Self::$name(widget) => widget.handle_event(event, cx),
                    )+
                }
            }

            fn on_click(&mut self) {
                match self {
                    $(
                        Self::$name(widget) => widget.on_click(),
                    )+
                }
            }

            fn text(&self) -> Option<TextContent> {
                match self {
                    $(
                        Self::$name(widget) => widget.text(),
                    )+
                }
            }
        }

    };
}

define_widgets! {
    Container => ContainerWidget,
    Column => ColumnWidget,
    Row => RowWidget,
    Label => LabelWidget,
    Text => TextWidget,
    Button => ButtonWidget,
    StyleScope => StyleScopeWidget,
    ScrollScope => ScrollScope,
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

    pub(crate) fn style_animation_rules(&self) -> Vec<StyleAnimationRule> {
        self.with_widgets(|widget| {
            let mut rules = widget
                .style_animations()
                .iter()
                .map(StyleAnimationRule::from_style_animation)
                .collect::<Vec<_>>();
            if let Widgets::Button(button) = widget {
                rules.extend(button.state_style_animation_rules());
            }
            rules
        })
    }

    pub fn take_event_handlers(&self) -> EventHandlers {
        self.with_widgets_mut(|widget| std::mem::take(widget.event_handlers_mut()))
    }

    pub fn paint(&self, rect: Rect, style: &ComputedStyle, commands: &mut Vec<PaintCommand>) {
        self.with_widgets(|widget| widget.paint(rect, style, commands));
    }

    pub fn handle_event(
        &self,
        event: &xui_interface::Event,
        cx: &mut xui_interface::EventContext<'_>,
    ) -> xui_interface::EventResult {
        self.with_widgets_mut(|widget| widget.handle_event(event, cx))
    }

    pub fn on_click(&self) {
        self.with_widgets_mut(|widget| widget.on_click());
    }

    pub fn update_from(&self, next: &Self) -> xui_interface::DirtyFlags {
        self.with_widgets_mut(|current| next.with_widgets(|next| current.update_from(next)))
    }

    pub fn computed_style(
        &self,
        parent: &ComputedStyle,
        theme: &crate::style::Theme,
    ) -> ComputedStyle {
        crate::layout::computed_style_for_widget(self, parent, theme)
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
        self.props = Some(Box::new(props));
        self
    }

    pub fn props_with_hash<P>(mut self, props: P, props_hash: u64) -> Self
    where
        P: Any,
    {
        self.props_hash = props_hash;
        self.props = Some(Box::new(props));
        self
    }
}

pub fn label(text: impl Into<TextContent>) -> LabelWidget {
    LabelWidget::new(text)
}

pub fn text(text: impl Into<xui_interface::TextContent>) -> TextWidget {
    TextWidget::new(text)
}

pub fn button(text: impl Into<TextContent>) -> ButtonWidget {
    ButtonWidget::new(text)
}

pub fn column() -> ColumnWidget {
    ColumnWidget::new()
}

pub fn row() -> RowWidget {
    RowWidget::new()
}

pub fn container() -> ContainerWidget {
    ContainerWidget::new()
}

pub fn style_scope(style: Style) -> StyleScopeWidget {
    StyleScopeWidget::new(style)
}

pub fn scroll_scope() -> ScrollScope {
    ScrollScope::new()
}

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
