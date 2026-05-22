use std::any::Any;
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use taffy::prelude as tf;
pub use xui_interface::{EventHandlers, Widget, WidgetType};

use crate::core::{Color, EdgeInsets, Size};
use crate::fiber::{ComponentType, ErasedProps, Key};
use crate::font::TextI;
use crate::state::HookContext;

macro_rules! event_handler_methods {
    () => {
        pub fn on_event(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::Event,
                &mut xui_interface::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_event = Some(Box::new(handler));
            self
        }

        pub fn on_click(
            mut self,
            handler: impl for<'a> FnMut(
                &mut xui_interface::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_click = Some(Box::new(handler));
            self
        }

        pub fn on_hover_change(
            mut self,
            handler: impl for<'a> FnMut(
                bool,
                &mut xui_interface::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_hover_change = Some(Box::new(handler));
            self
        }

        pub fn on_pointer_down(
            mut self,
            handler: impl for<'a> FnMut(
                &mut xui_interface::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_pointer_down = Some(Box::new(handler));
            self
        }

        pub fn on_pointer_up(
            mut self,
            handler: impl for<'a> FnMut(
                &mut xui_interface::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_pointer_up = Some(Box::new(handler));
            self
        }

        pub fn on_pointer_move(
            mut self,
            handler: impl for<'a> FnMut(
                &mut xui_interface::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_pointer_move = Some(Box::new(handler));
            self
        }

        pub fn on_key_down(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::InputKey,
                &mut xui_interface::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_key_down = Some(Box::new(handler));
            self
        }

        pub fn on_key_up(
            mut self,
            handler: impl for<'a> FnMut(
                &xui_interface::InputKey,
                &mut xui_interface::EventContext<'a>,
            ) -> xui_interface::EventResult
            + 'static,
        ) -> Self {
            self.event_handlers.on_key_up = Some(Box::new(handler));
            self
        }
    };
}

mod button;
mod column;
mod container;
mod label;
mod root;
mod row;

pub use button::ButtonWidget;
pub use column::ColumnWidget;
pub use container::ContainerWidget;
pub use label::LabelWidget;
pub use row::RowWidget;

pub type ComponentRender = Rc<RefCell<dyn for<'a> FnMut(&mut HookContext<'a>) -> Element>>;

#[derive(Clone, Debug)]
pub struct WidgetRef {
    widget: Rc<RefCell<Box<dyn Widget>>>,
}

impl WidgetRef {
    pub fn new(widget: impl Widget + 'static) -> Self {
        Self::from(Box::new(widget) as Box<dyn Widget>)
    }

    pub fn with<R>(&self, f: impl FnOnce(&dyn Widget) -> R) -> R {
        let widget = self.widget.borrow();
        f(&**widget)
    }

    pub fn with_mut<R>(&self, f: impl FnOnce(&mut dyn Widget) -> R) -> R {
        let mut widget = self.widget.borrow_mut();
        f(&mut **widget)
    }
}

impl From<Box<dyn Widget>> for WidgetRef {
    fn from(widget: Box<dyn Widget>) -> Self {
        Self {
            widget: Rc::new(RefCell::new(widget)),
        }
    }
}

pub struct HostElement {
    widget: WidgetRef,
    children: Vec<Element>,
}

pub enum Element {
    Host(HostElement),
    Component(ComponentElement),
}

pub struct HostParts {
    pub widget: WidgetRef,
    pub event_handlers: EventHandlers,
    pub children: Vec<Element>,
}

impl HostElement {
    pub fn new(widget: impl Widget + 'static, children: Vec<Element>) -> Self {
        Self {
            widget: WidgetRef::new(widget),
            children,
        }
    }

    pub fn key(&self) -> Option<Key> {
        self.widget.with(|widget| widget.key().cloned())
    }

    pub fn node_type(&self) -> WidgetType {
        self.widget.with(|widget| widget.node_type())
    }

    pub fn props_hash(&self) -> u64 {
        self.widget.with(|widget| widget.props_hash())
    }

    pub fn style(&self, measurer: &mut TextI) -> tf::Style {
        self.widget
            .with(|widget| style_for_widget(widget, measurer))
    }

    pub fn into_parts(self) -> HostParts {
        let event_handlers = self
            .widget
            .with_mut(|widget| std::mem::take(widget.event_handlers_mut()));
        HostParts {
            widget: self.widget,
            event_handlers,
            children: self.children,
        }
    }

    pub fn children_mut(&mut self) -> &mut Vec<Element> {
        &mut self.children
    }
}

impl Element {
    pub fn key(&self) -> Option<Key> {
        match self {
            Self::Host(host) => host.key(),
            Self::Component(component) => component.key.clone(),
        }
    }

    pub fn node_type(&self) -> Option<WidgetType> {
        match self {
            Self::Host(host) => Some(host.node_type()),
            Self::Component(_) => None,
        }
    }

    pub fn props_hash(&self) -> u64 {
        match self {
            Self::Host(host) => host.props_hash(),
            Self::Component(widget) => {
                let mut hasher = DefaultHasher::new();
                self.node_type().hash(&mut hasher);
                self.key().hash(&mut hasher);
                widget.render.hash(&mut hasher);
                widget.props_hash.hash(&mut hasher);
                hasher.finish()
            }
        }
    }

    pub fn style(&self, measurer: &mut TextI) -> tf::Style {
        match self {
            Self::Host(host) => host.style(measurer),
            Self::Component(_) => panic!("component elements do not have layout style"),
        }
    }

    pub fn into_parts(self) -> HostParts {
        match self {
            Self::Host(host) => host.into_parts(),
            Self::Component(_) => panic!("component elements do not have widget parts"),
        }
    }

    pub fn children_mut(&mut self) -> Option<&mut Vec<Element>> {
        match self {
            Self::Host(host) => Some(host.children_mut()),
            Self::Component(_) => None,
        }
    }
}

pub struct ComponentElement {
    pub key: Option<Key>,
    pub render: ComponentType,
    pub props: ErasedProps,
    pub props_hash: u64,
}

impl ComponentElement {
    pub fn new(component_type: ComponentType) -> Self {
        Self {
            key: None,
            render: component_type,
            props: Rc::new(()),
            props_hash: props_hash(&()),
        }
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn props<P>(mut self, props: P) -> Self
    where
        P: Any + Hash,
    {
        self.props_hash = props_hash(&props);
        self.props = Rc::new(props);
        self
    }

    pub fn props_with_hash<P>(mut self, props: P, props_hash: u64) -> Self
    where
        P: Any,
    {
        self.props_hash = props_hash;
        self.props = Rc::new(props);
        self
    }
}

pub fn label(text: impl Into<String>) -> LabelWidget {
    LabelWidget::new(text)
}

pub fn button(text: impl Into<String>) -> ButtonWidget {
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

pub fn component(render: ComponentType) -> ComponentElement {
    ComponentElement::new(render)
}

impl From<LabelWidget> for Element {
    fn from(value: LabelWidget) -> Self {
        Self::Host(HostElement::new(value, Vec::new()))
    }
}

impl From<ButtonWidget> for Element {
    fn from(value: ButtonWidget) -> Self {
        Self::Host(HostElement::new(value, Vec::new()))
    }
}

impl From<ColumnWidget> for Element {
    fn from(mut value: ColumnWidget) -> Self {
        let children = std::mem::take(&mut value.children);
        Self::Host(HostElement::new(value, children))
    }
}

impl From<RowWidget> for Element {
    fn from(mut value: RowWidget) -> Self {
        let children = std::mem::take(&mut value.children);
        Self::Host(HostElement::new(value, children))
    }
}

impl From<ContainerWidget> for Element {
    fn from(mut value: ContainerWidget) -> Self {
        let children = std::mem::take(&mut value.children);
        Self::Host(HostElement::new(value, children))
    }
}

impl From<ComponentElement> for Element {
    fn from(value: ComponentElement) -> Self {
        Self::Component(value)
    }
}

impl From<HostElement> for Element {
    fn from(value: HostElement) -> Self {
        Self::Host(value)
    }
}

pub fn root_widget() -> Box<dyn Widget> {
    Box::new(root::RootWidget::default())
}

pub fn style_for_widget(widget: &dyn Widget, measurer: &mut TextI) -> tf::Style {
    if let Some(label) = widget.as_any().downcast_ref::<LabelWidget>() {
        return label
            .measure(measurer)
            .map(fixed_size_style)
            .unwrap_or_default();
    }

    if let Some(button) = widget.as_any().downcast_ref::<ButtonWidget>() {
        return button
            .measure(measurer)
            .map(fixed_size_style)
            .unwrap_or_default();
    }

    if let Some(column) = widget.as_any().downcast_ref::<ColumnWidget>() {
        return tf::Style {
            display: tf::Display::Flex,
            flex_direction: tf::FlexDirection::Column,
            gap: tf::Size {
                width: length_percentage(column.gap),
                height: length_percentage(column.gap),
            },
            ..Default::default()
        };
    }

    if let Some(row) = widget.as_any().downcast_ref::<RowWidget>() {
        return tf::Style {
            display: tf::Display::Flex,
            flex_direction: tf::FlexDirection::Row,
            gap: tf::Size {
                width: length_percentage(row.gap),
                height: length_percentage(row.gap),
            },
            ..Default::default()
        };
    }

    if let Some(container) = widget.as_any().downcast_ref::<ContainerWidget>() {
        let mut style = tf::Style {
            display: tf::Display::Flex,
            flex_direction: tf::FlexDirection::Column,
            padding: edge_insets(container.padding),
            ..Default::default()
        };

        if let Some(size) = container.size {
            style.size = tf::Size {
                width: dimension(size.width),
                height: dimension(size.height),
            };
        }

        return style;
    }

    tf::Style::default()
}

fn fixed_size_style(size: Size) -> tf::Style {
    tf::Style {
        size: tf::Size {
            width: dimension(size.width),
            height: dimension(size.height),
        },
        ..Default::default()
    }
}

fn edge_insets(value: EdgeInsets) -> tf::Rect<tf::LengthPercentage> {
    tf::Rect {
        left: length_percentage(value.left),
        right: length_percentage(value.right),
        top: length_percentage(value.top),
        bottom: length_percentage(value.bottom),
    }
}

fn dimension(value: f32) -> tf::Dimension {
    tf::Dimension::length(value)
}

fn length_percentage(value: f32) -> tf::LengthPercentage {
    tf::LengthPercentage::length(value)
}

pub(super) fn props_hash<T: Hash>(props: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    props.hash(&mut hasher);
    hasher.finish()
}

pub(super) fn hash_color<H: Hasher>(color: Color, hasher: &mut H) {
    color.r.to_bits().hash(hasher);
    color.g.to_bits().hash(hasher);
    color.b.to_bits().hash(hasher);
    color.a.to_bits().hash(hasher);
}

pub(super) fn hash_edge_insets<H: Hasher>(value: EdgeInsets, hasher: &mut H) {
    value.left.to_bits().hash(hasher);
    value.right.to_bits().hash(hasher);
    value.top.to_bits().hash(hasher);
    value.bottom.to_bits().hash(hasher);
}
