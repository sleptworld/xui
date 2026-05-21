mod button;
mod column;
mod container;
mod label;
mod root;
mod row;

use std::any::Any;
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

use taffy::prelude as tf;
pub use xui_components::{Button, Label};
pub use xui_interface::{EventHandlers, Widget, WidgetKind, WidgetType};

pub use button::ButtonWidget;
pub use column::ColumnWidget;
pub use container::ContainerWidget;
pub use label::LabelWidget;
pub use row::RowWidget;

use crate::core::{Color, EdgeInsets, Size};
use crate::fiber::{ComponentType, ErasedProps, Key};
use crate::font::TextI;
use crate::state::HookContext;

pub type Column = xui_components::Column<Element>;
pub type Row = xui_components::Row<Element>;
pub type Container = xui_components::Container<Element>;

pub type ComponentRender = Rc<RefCell<dyn for<'a> FnMut(&mut HookContext<'a>) -> Element>>;

#[derive(Debug)]
pub struct WidgetRef {
    widget: Box<dyn Widget>,
}

impl Deref for WidgetRef {
    type Target = dyn Widget;

    fn deref(&self) -> &Self::Target {
        &*self.widget
    }
}

impl From<Box<dyn Widget>> for WidgetRef {
    fn from(widget: Box<dyn Widget>) -> Self {
        Self { widget }
    }
}

impl DerefMut for WidgetRef {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.widget
    }
}

pub enum HostElement {
    Label(Label),
    Button(Button),
    Column(Column),
    Row(Row),
    Container(Container),
}

pub enum Element {
    Host(HostElement),
    Component(ComponentElement),
}

pub struct HostParts {
    pub kind: WidgetKind,
    pub widget: WidgetRef,
    pub event_handlers: EventHandlers,
    pub children: Vec<Element>,
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
            Self::Host(host) => host.children_mut(),
            Self::Component(_) => None,
        }
    }
}

impl HostElement {
    pub fn key(&self) -> Option<Key> {
        match self {
            Self::Label(widget) => widget.key.clone(),
            Self::Button(widget) => widget.key.clone(),
            Self::Column(widget) => widget.key.clone(),
            Self::Row(widget) => widget.key.clone(),
            Self::Container(widget) => widget.key.clone(),
        }
    }

    pub fn node_type(&self) -> WidgetType {
        match self {
            Self::Label(_) => WidgetType::Label,
            Self::Button(_) => WidgetType::Button,
            Self::Column(_) => WidgetType::Column,
            Self::Row(_) => WidgetType::Row,
            Self::Container(_) => WidgetType::Container,
        }
    }

    pub fn props_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.node_type().hash(&mut hasher);
        self.key().hash(&mut hasher);
        match self {
            Self::Label(widget) => {
                widget.text.hash(&mut hasher);
                hash_color(widget.color, &mut hasher);
                widget.font_size.to_bits().hash(&mut hasher);
            }
            Self::Button(widget) => {
                widget.text.hash(&mut hasher);
            }
            Self::Column(widget) => {
                widget.gap.to_bits().hash(&mut hasher);
            }
            Self::Row(widget) => {
                widget.gap.to_bits().hash(&mut hasher);
            }
            Self::Container(widget) => {
                widget
                    .size
                    .map(|size| (size.width.to_bits(), size.height.to_bits()))
                    .hash(&mut hasher);
                hash_edge_insets(widget.padding, &mut hasher);
                hash_color(widget.background, &mut hasher);
            }
        }
        hasher.finish()
    }

    pub fn style(&self, measurer: &mut TextI) -> tf::Style {
        match self {
            Self::Label(label) => {
                let widget = LabelWidget {
                    text: label.text.clone(),
                    color: label.color,
                    font_size: label.font_size,
                };
                widget
                    .measure(measurer)
                    .map(fixed_size_style)
                    .unwrap_or_default()
            }
            Self::Button(button) => {
                let widget = ButtonWidget {
                    text: button.text.clone(),
                    pressed: false,
                    hovered: false,
                };
                widget
                    .measure(measurer)
                    .map(fixed_size_style)
                    .unwrap_or_default()
            }
            Self::Column(column) => tf::Style {
                display: tf::Display::Flex,
                flex_direction: tf::FlexDirection::Column,
                gap: tf::Size {
                    width: length_percentage(column.gap),
                    height: length_percentage(column.gap),
                },
                ..Default::default()
            },
            Self::Row(row) => tf::Style {
                display: tf::Display::Flex,
                flex_direction: tf::FlexDirection::Row,
                gap: tf::Size {
                    width: length_percentage(row.gap),
                    height: length_percentage(row.gap),
                },
                ..Default::default()
            },
            Self::Container(container) => {
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

                style
            }
        }
    }

    pub fn into_parts(self) -> HostParts {
        match self {
            Self::Label(widget) => {
                let kind = WidgetKind::Label {
                    text: widget.text,
                    color: widget.color,
                    font_size: widget.font_size,
                };
                HostParts {
                    kind: kind.clone(),
                    widget: widget_from_kind(kind).into(),
                    event_handlers: widget.event_handlers,
                    children: Vec::new(),
                }
            }
            Self::Button(widget) => {
                let kind = WidgetKind::Button {
                    text: widget.text,
                    pressed: false,
                    hovered: false,
                };
                HostParts {
                    kind: kind.clone(),
                    widget: widget_from_kind(kind).into(),
                    event_handlers: widget.event_handlers,
                    children: Vec::new(),
                }
            }
            Self::Column(widget) => {
                let kind = WidgetKind::Column { gap: widget.gap };
                HostParts {
                    kind: kind.clone(),
                    widget: widget_from_kind(kind).into(),
                    event_handlers: widget.event_handlers,
                    children: widget.children,
                }
            }
            Self::Row(widget) => {
                let kind = WidgetKind::Row { gap: widget.gap };
                HostParts {
                    kind: kind.clone(),
                    widget: widget_from_kind(kind).into(),
                    event_handlers: widget.event_handlers,
                    children: widget.children,
                }
            }
            Self::Container(widget) => {
                let kind = WidgetKind::Container {
                    background: widget.background,
                };
                HostParts {
                    kind: kind.clone(),
                    widget: widget_from_kind(kind).into(),
                    event_handlers: widget.event_handlers,
                    children: widget.children,
                }
            }
        }
    }

    pub fn children_mut(&mut self) -> Option<&mut Vec<Element>> {
        match self {
            Self::Column(widget) => Some(&mut widget.children),
            Self::Row(widget) => Some(&mut widget.children),
            Self::Container(widget) => Some(&mut widget.children),
            _ => None,
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

pub fn label(text: impl Into<String>) -> Label {
    Label::new(text)
}

pub fn button(text: impl Into<String>) -> Button {
    Button::new(text)
}

pub fn column() -> Column {
    xui_components::column()
}

pub fn row() -> Row {
    xui_components::row()
}

pub fn container() -> Container {
    xui_components::container()
}

pub fn component(render: ComponentType) -> ComponentElement {
    ComponentElement::new(render)
}

fn props_hash<T: Hash>(props: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    props.hash(&mut hasher);
    hasher.finish()
}

impl From<Label> for Element {
    fn from(value: Label) -> Self {
        Self::Host(HostElement::Label(value))
    }
}

impl From<Button> for Element {
    fn from(value: Button) -> Self {
        Self::Host(HostElement::Button(value))
    }
}

impl From<Column> for Element {
    fn from(value: Column) -> Self {
        Self::Host(HostElement::Column(value))
    }
}

impl From<Row> for Element {
    fn from(value: Row) -> Self {
        Self::Host(HostElement::Row(value))
    }
}

impl From<Container> for Element {
    fn from(value: Container) -> Self {
        Self::Host(HostElement::Container(value))
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

pub fn widget_from_kind(kind: WidgetKind) -> Box<dyn Widget> {
    match kind {
        WidgetKind::Root => Box::new(root::RootWidget),
        WidgetKind::Label {
            text,
            color,
            font_size,
        } => Box::new(LabelWidget {
            text,
            color,
            font_size,
        }),
        WidgetKind::Button {
            text,
            pressed,
            hovered,
        } => Box::new(ButtonWidget {
            text,
            pressed,
            hovered,
        }),
        WidgetKind::Column { gap } => Box::new(ColumnWidget { gap }),
        WidgetKind::Row { gap } => Box::new(RowWidget { gap }),
        WidgetKind::Container { background } => Box::new(ContainerWidget { background }),
    }
}

pub fn update_kind_from(kind: &mut WidgetKind, new_kind: WidgetKind) -> xui_interface::DirtyFlags {
    if kind.node_type() != new_kind.node_type() {
        *kind = new_kind;
        return xui_interface::DirtyFlags::TREE
            | xui_interface::DirtyFlags::LAYOUT
            | xui_interface::DirtyFlags::PAINT;
    }

    match (kind, new_kind) {
        (
            WidgetKind::Label {
                text,
                color,
                font_size,
            },
            WidgetKind::Label {
                text: new_text,
                color: new_color,
                font_size: new_font_size,
            },
        ) => {
            let mut flags = xui_interface::DirtyFlags::empty();
            if *text != new_text {
                *text = new_text;
                flags |= xui_interface::DirtyFlags::LAYOUT | xui_interface::DirtyFlags::PAINT;
            }
            if *font_size != new_font_size {
                *font_size = new_font_size;
                flags |= xui_interface::DirtyFlags::LAYOUT | xui_interface::DirtyFlags::PAINT;
            }
            if *color != new_color {
                *color = new_color;
                flags |= xui_interface::DirtyFlags::PAINT;
            }
            flags
        }
        (WidgetKind::Button { text, .. }, WidgetKind::Button { text: new_text, .. }) => {
            if *text != new_text {
                *text = new_text;
                xui_interface::DirtyFlags::LAYOUT | xui_interface::DirtyFlags::PAINT
            } else {
                xui_interface::DirtyFlags::empty()
            }
        }
        (WidgetKind::Column { gap }, WidgetKind::Column { gap: new_gap }) => {
            if *gap != new_gap {
                *gap = new_gap;
                xui_interface::DirtyFlags::STYLE
                    | xui_interface::DirtyFlags::LAYOUT
                    | xui_interface::DirtyFlags::PAINT
            } else {
                xui_interface::DirtyFlags::empty()
            }
        }
        (WidgetKind::Row { gap }, WidgetKind::Row { gap: new_gap }) => {
            if *gap != new_gap {
                *gap = new_gap;
                xui_interface::DirtyFlags::STYLE
                    | xui_interface::DirtyFlags::LAYOUT
                    | xui_interface::DirtyFlags::PAINT
            } else {
                xui_interface::DirtyFlags::empty()
            }
        }
        (
            WidgetKind::Container { background },
            WidgetKind::Container {
                background: new_background,
            },
        ) => {
            if *background != new_background {
                *background = new_background;
                xui_interface::DirtyFlags::PAINT
            } else {
                xui_interface::DirtyFlags::empty()
            }
        }
        _ => xui_interface::DirtyFlags::empty(),
    }
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

fn hash_color(color: Color, hasher: &mut DefaultHasher) {
    color.r.to_bits().hash(hasher);
    color.g.to_bits().hash(hasher);
    color.b.to_bits().hash(hasher);
    color.a.to_bits().hash(hasher);
}

fn hash_edge_insets(value: EdgeInsets, hasher: &mut DefaultHasher) {
    value.left.to_bits().hash(hasher);
    value.right.to_bits().hash(hasher);
    value.top.to_bits().hash(hasher);
    value.bottom.to_bits().hash(hasher);
}
