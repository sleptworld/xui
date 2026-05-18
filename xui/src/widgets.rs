use std::any::TypeId;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::rc::Rc;

use taffy::prelude as tf;
pub use xui_components::{
    Button, Key, Label,  Widget, WidgetKind, button, update_kind_from, widget_from_kind,
};
pub use xui_interface::{Event, EventContext, EventResult, WidgetType};

use crate::core::{Color, EdgeInsets};
use crate::font::TextI;
use crate::layout::style_for_element;
use crate::state::HookContext;

pub type Column = xui_components::Column<Element>;
pub type Row = xui_components::Row<Element>;
pub type Container = xui_components::Container<Element>;

pub type EventHandler = Box<dyn FnMut(&Event, &mut EventContext<'_>) -> EventResult>;
#[derive(Clone, Debug)]
pub struct WidgetRef {
    widget: Rc<dyn Widget>
}

impl Deref for WidgetRef {
    type Target = dyn Widget;
    fn deref(&self) -> &Self::Target {
        &*self.widget
    }
}

pub fn key_from_hash<T: Hash>(value: &T) -> Key {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    Key(hasher.finish().to_string())
}

pub enum Element {
    Label(Label),
    Button(Button),
    Column(Column),
    Row(Row),
    Container(Container),
    Component(ComponentElement),
}

impl Element {
    pub fn key(&self) -> Option<Key> {
        match self {
            Self::Label(widget) => widget.key.clone(),
            Self::Button(widget) => widget.key.clone(),
            Self::Column(widget) => widget.key.clone(),
            Self::Row(widget) => widget.key.clone(),
            Self::Container(widget) => widget.key.clone(),
            Self::Component(component) => component.key.clone(),
        }
    }

    pub fn node_type(&self) -> Option<WidgetType> {
        match self {
            Self::Label(_) => Some(WidgetType::Label),
            Self::Button(_) => Some(WidgetType::Button),
            Self::Column(_) => Some(WidgetType::Column),
            Self::Row(_) => Some(WidgetType::Row),
            Self::Container(_) => Some(WidgetType::Container),
            Self::Component(_) => None,
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
            Self::Component(widget) => {
                widget.type_id.hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    pub fn style(&self, measurer: &mut TextI) -> tf::Style {
        style_for_element(self, measurer)
    }

    pub fn into_parts(self) -> (WidgetKind, Box<dyn Widget>, Vec<Element>) {
        match self {
            Self::Label(widget) => {
                let kind = WidgetKind::Label {
                    text: widget.text,
                    color: widget.color,
                    font_size: widget.font_size,
                };
                (kind.clone(), widget_from_kind(kind, None), Vec::new())
            }
            Self::Button(mut widget) => {
                let kind = WidgetKind::Button {
                    text: widget.text,
                    pressed: false,
                    hovered: false,
                };
                (
                    kind.clone(),
                    widget_from_kind(kind, widget.on_click.take()),
                    Vec::new(),
                )
            }
            Self::Column(widget) => {
                let kind = WidgetKind::Column { gap: widget.gap };
                (kind.clone(), widget_from_kind(kind, None), widget.children)
            }
            Self::Row(widget) => {
                let kind = WidgetKind::Row { gap: widget.gap };
                (kind.clone(), widget_from_kind(kind, None), widget.children)
            }
            Self::Container(widget) => {
                let kind = WidgetKind::Container {
                    background: widget.background,
                };
                (kind.clone(), widget_from_kind(kind, None), widget.children)
            }
            Self::Component(_) => panic!("component elements do not have widget parts"),
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
    pub type_id: TypeId,
    pub render: Box<dyn FnMut(&mut HookContext<'_>) -> Element>,
}

impl ComponentElement {
    pub fn new<F>(render: F) -> Self
    where
        F: FnMut(&mut HookContext<'_>) -> Element + 'static,
    {
        Self {
            key: None,
            type_id: TypeId::of::<F>(),
            render: Box::new(render),
        }
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

pub fn label(text: impl Into<String>) -> Label {
    Label::new(text)
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

pub fn component<F>(render: F) -> ComponentElement
where
    F: FnMut(&mut HookContext<'_>) -> Element + 'static,
{
    ComponentElement::new(render)
}

impl From<Label> for Element {
    fn from(value: Label) -> Self {
        Self::Label(value)
    }
}

impl From<Button> for Element {
    fn from(value: Button) -> Self {
        Self::Button(value)
    }
}

impl From<Column> for Element {
    fn from(value: Column) -> Self {
        Self::Column(value)
    }
}

impl From<Row> for Element {
    fn from(value: Row) -> Self {
        Self::Row(value)
    }
}

impl From<Container> for Element {
    fn from(value: Container) -> Self {
        Self::Container(value)
    }
}

impl From<ComponentElement> for Element {
    fn from(value: ComponentElement) -> Self {
        Self::Component(value)
    }
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
