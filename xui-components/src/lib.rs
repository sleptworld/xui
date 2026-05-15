pub mod widgets;
use std::hash::{Hash, Hasher};
use widgets::{ButtonWidget, ColumnWidget, ContainerWidget, LabelWidget, RootWidget, RowWidget};
use xui_interface::{Color, DirtyFlags, EdgeInsets, Event, EventContext, EventResult, Size};
pub use xui_interface::{Key, Widget, WidgetKind, WidgetType};

pub type EventHandler = Box<dyn FnMut(&Event, &mut EventContext<'_>) -> EventResult>;

#[derive(Debug, Clone)]
pub struct Label {
    pub key: Option<Key>,
    pub text: String,
    pub color: Color,
    pub font_size: f32,
}

impl Label {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            key: None,
            color: Color::BLACK,
            font_size: 14.0,
        }
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

pub struct Button {
    pub key: Option<Key>,
    pub text: String,
    pub on_click: Option<Box<dyn FnMut()>>,
}

impl Button {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            key: None,
            on_click: None,
        }
    }

    pub fn on_click(mut self, handler: impl FnMut() + 'static) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

pub struct Column<Child = ()> {
    pub key: Option<Key>,
    pub children: Vec<Child>,
    pub gap: f32,
}

impl<Child> Column<Child> {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
            key: None,
            gap: 0.0,
        }
    }

    pub fn child(mut self, child: impl Into<Child>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl<Child> Default for Column<Child> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Row<Child = ()> {
    pub key: Option<Key>,
    pub children: Vec<Child>,
    pub gap: f32,
}

impl<Child> Row<Child> {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
            key: None,
            gap: 0.0,
        }
    }

    pub fn child(mut self, child: impl Into<Child>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl<Child> Default for Row<Child> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Container<Child = ()> {
    pub key: Option<Key>,
    pub children: Vec<Child>,
    pub size: Option<Size>,
    pub padding: EdgeInsets,
    pub background: Color,
}

impl<Child> Container<Child> {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
            key: None,
            size: None,
            padding: EdgeInsets::ZERO,
            background: Color::TRANSPARENT,
        }
    }

    pub fn child(mut self, child: impl Into<Child>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn size(mut self, size: Size) -> Self {
        self.size = Some(size);
        self
    }

    pub fn padding(mut self, padding: EdgeInsets) -> Self {
        self.padding = padding;
        self
    }

    pub fn background(mut self, background: Color) -> Self {
        self.background = background;
        self
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl<Child> Default for Container<Child> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
pub struct ContainerProps {
    pub size: Option<Size>,
    pub padding: EdgeInsets,
    pub background: Color,
}

impl Default for ContainerProps {
    fn default() -> Self {
        Self {
            size: None,
            padding: EdgeInsets::ZERO,
            background: Color::TRANSPARENT,
        }
    }
}

impl Hash for ContainerProps {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.size
            .map(|size| (size.width.to_bits(), size.height.to_bits()))
            .hash(state);
        hash_edge_insets(self.padding, state);
        hash_color(self.background, state);
    }
}

#[derive(Clone, Debug, Default)]
pub struct ColumnProps {
    pub gap: f32,
}

impl Hash for ColumnProps {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.gap.to_bits().hash(state);
    }
}

#[derive(Clone, Debug, Default)]
pub struct RowProps {
    pub gap: f32,
}

impl Hash for RowProps {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.gap.to_bits().hash(state);
    }
}

pub fn label(text: impl Into<String>) -> Label {
    Label::new(text)
}

pub fn button(text: impl Into<String>) -> Button {
    Button::new(text)
}

pub fn column<Child>() -> Column<Child> {
    Column::new()
}

pub fn row<Child>() -> Row<Child> {
    Row::new()
}

pub fn container<Child>() -> Container<Child> {
    Container::new()
}

pub fn container_component<Child>(
    _cx: &mut impl Sized,
    props: ContainerProps,
    children: Vec<Child>,
) -> Container<Child> {
    let mut element = Container::new()
        .padding(props.padding)
        .background(props.background);

    if let Some(size) = props.size {
        element = element.size(size);
    }

    for child in children {
        element = element.child(child);
    }

    element
}

pub fn column_component<Child>(
    _cx: &mut impl Sized,
    props: ColumnProps,
    children: Vec<Child>,
) -> Column<Child> {
    let mut element = Column::new().gap(props.gap);

    for child in children {
        element = element.child(child);
    }

    element
}

pub fn row_component<Child>(
    _cx: &mut impl Sized,
    props: RowProps,
    children: Vec<Child>,
) -> Row<Child> {
    let mut element = Row::new().gap(props.gap);

    for child in children {
        element = element.child(child);
    }

    element
}

pub fn widget_from_kind(kind: WidgetKind, on_click: Option<Box<dyn FnMut()>>) -> Box<dyn Widget> {
    match kind {
        WidgetKind::Root => Box::new(RootWidget),
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
            on_click,
        }),
        WidgetKind::Column { gap } => Box::new(ColumnWidget { gap }),
        WidgetKind::Row { gap } => Box::new(RowWidget { gap }),
        WidgetKind::Container { background } => Box::new(ContainerWidget { background }),
    }
}

pub fn update_kind_from(kind: &mut WidgetKind, new_kind: WidgetKind) -> DirtyFlags {
    if kind.node_type() != new_kind.node_type() {
        *kind = new_kind;
        return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
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
            let mut flags = DirtyFlags::empty();
            if *text != new_text {
                *text = new_text;
                flags |= DirtyFlags::LAYOUT | DirtyFlags::PAINT;
            }
            if *font_size != new_font_size {
                *font_size = new_font_size;
                flags |= DirtyFlags::LAYOUT | DirtyFlags::PAINT;
            }
            if *color != new_color {
                *color = new_color;
                flags |= DirtyFlags::PAINT;
            }
            flags
        }
        (WidgetKind::Button { text, .. }, WidgetKind::Button { text: new_text, .. }) => {
            if *text != new_text {
                *text = new_text;
                DirtyFlags::LAYOUT | DirtyFlags::PAINT
            } else {
                DirtyFlags::empty()
            }
        }
        (WidgetKind::Column { gap }, WidgetKind::Column { gap: new_gap }) => {
            if *gap != new_gap {
                *gap = new_gap;
                DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT
            } else {
                DirtyFlags::empty()
            }
        }
        (WidgetKind::Row { gap }, WidgetKind::Row { gap: new_gap }) => {
            if *gap != new_gap {
                *gap = new_gap;
                DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT
            } else {
                DirtyFlags::empty()
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
                DirtyFlags::PAINT
            } else {
                DirtyFlags::empty()
            }
        }
        _ => DirtyFlags::empty(),
    }
}

fn hash_color<H: Hasher>(color: Color, state: &mut H) {
    color.r.to_bits().hash(state);
    color.g.to_bits().hash(state);
    color.b.to_bits().hash(state);
    color.a.to_bits().hash(state);
}

fn hash_edge_insets<H: Hasher>(value: EdgeInsets, state: &mut H) {
    value.left.to_bits().hash(state);
    value.right.to_bits().hash(state);
    value.top.to_bits().hash(state);
    value.bottom.to_bits().hash(state);
}
