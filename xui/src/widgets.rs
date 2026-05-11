use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use taffy::prelude as tf;
use xui_interface::{DirtyFlags, Event, EventContext, EventResult, PointerButton, TextMeasurer};
pub use xui_interface::{Key, NodeType, Widget, WidgetKind};

use crate::core::{Color, EdgeInsets, Point, Rect, Size};
use crate::layout::style_for_element;
use crate::render::PaintCommand;

pub type EventHandler = Box<dyn FnMut(&Event, &mut EventContext<'_>) -> EventResult>;

pub enum Element {
    Label(Label),
    Button(Button),
    Column(Column),
    Row(Row),
    Container(Container),
}

impl Element {
    pub fn key(&self) -> Option<Key> {
        match self {
            Self::Label(widget) => widget.key.clone(),
            Self::Button(widget) => widget.key.clone(),
            Self::Column(widget) => widget.key.clone(),
            Self::Row(widget) => widget.key.clone(),
            Self::Container(widget) => widget.key.clone(),
        }
    }

    pub fn node_type(&self) -> NodeType {
        match self {
            Self::Label(_) => NodeType::Label,
            Self::Button(_) => NodeType::Button,
            Self::Column(_) => NodeType::Column,
            Self::Row(_) => NodeType::Row,
            Self::Container(_) => NodeType::Container,
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

    pub fn style(&self, measurer: &dyn TextMeasurer) -> tf::Style {
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

pub struct Column {
    pub key: Option<Key>,
    pub children: Vec<Element>,
    pub gap: f32,
}

impl Column {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
            key: None,
            gap: 0.0,
        }
    }

    pub fn child(mut self, child: impl Into<Element>) -> Self {
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

impl Default for Column {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Row {
    pub key: Option<Key>,
    pub children: Vec<Element>,
    pub gap: f32,
}

impl Row {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
            key: None,
            gap: 0.0,
        }
    }

    pub fn child(mut self, child: impl Into<Element>) -> Self {
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

impl Default for Row {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Container {
    pub key: Option<Key>,
    pub children: Vec<Element>,
    pub size: Option<Size>,
    pub padding: EdgeInsets,
    pub background: Color,
}

impl Container {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
            key: None,
            size: None,
            padding: EdgeInsets::ZERO,
            background: Color::TRANSPARENT,
        }
    }

    pub fn child(mut self, child: impl Into<Element>) -> Self {
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

impl Default for Container {
    fn default() -> Self {
        Self::new()
    }
}

pub fn label(text: impl Into<String>) -> Label {
    Label::new(text)
}

pub fn button(text: impl Into<String>) -> Button {
    Button::new(text)
}

pub fn column() -> Column {
    Column::new()
}

pub fn row() -> Row {
    Row::new()
}

pub fn container() -> Container {
    Container::new()
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

pub struct RootWidget;

impl Widget for RootWidget {
    fn node_type(&self) -> NodeType {
        NodeType::Root
    }

    fn update_from_kind(&mut self, _new_kind: &WidgetKind) -> DirtyFlags {
        DirtyFlags::empty()
    }

    fn paint(&self, _rect: Rect, _commands: &mut Vec<PaintCommand>) {}

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}

pub struct LabelWidget {
    pub text: String,
    pub color: Color,
    pub font_size: f32,
}

impl Widget for LabelWidget {
    fn node_type(&self) -> NodeType {
        NodeType::Label
    }

    fn update_from_kind(&mut self, new_kind: &WidgetKind) -> DirtyFlags {
        let WidgetKind::Label {
            text,
            color,
            font_size,
        } = new_kind
        else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };

        let mut flags = DirtyFlags::empty();
        if self.text != *text {
            self.text = text.clone();
            flags |= DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        }
        if self.font_size != *font_size {
            self.font_size = *font_size;
            flags |= DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        }
        if self.color != *color {
            self.color = *color;
            flags |= DirtyFlags::PAINT;
        }
        flags
    }

    fn paint(&self, rect: Rect, commands: &mut Vec<PaintCommand>) {
        commands.push(PaintCommand::Text {
            position: Point::new(rect.x, rect.y + self.font_size),
            text: self.text.clone(),
            color: self.color,
            size: self.font_size,
        });
    }

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}

pub struct ButtonWidget {
    pub text: String,
    pub pressed: bool,
    pub hovered: bool,
    pub on_click: Option<Box<dyn FnMut()>>,
}

impl Widget for ButtonWidget {
    fn node_type(&self) -> NodeType {
        NodeType::Button
    }

    fn update_from_kind(&mut self, new_kind: &WidgetKind) -> DirtyFlags {
        let WidgetKind::Button { text, .. } = new_kind else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };

        if self.text != *text {
            self.text = text.clone();
            DirtyFlags::LAYOUT | DirtyFlags::PAINT
        } else {
            DirtyFlags::empty()
        }
    }

    fn paint(&self, rect: Rect, commands: &mut Vec<PaintCommand>) {
        let background = if self.pressed {
            Color::BLUE_500
        } else if self.hovered {
            Color::GRAY_300
        } else {
            Color::GRAY_100
        };
        let text_color = if self.pressed {
            Color::WHITE
        } else {
            Color::BLACK
        };
        commands.push(PaintCommand::FillRect {
            rect,
            color: background,
        });
        commands.push(PaintCommand::StrokeRect {
            rect,
            color: Color::GRAY_300,
            width: 1.0,
        });
        commands.push(PaintCommand::Text {
            position: Point::new(rect.x + 8.0, rect.y + 18.0),
            text: self.text.clone(),
            color: text_color,
            size: 14.0,
        });
    }

    fn handle_event(&mut self, event: &Event, cx: &mut EventContext<'_>) -> EventResult {
        match event {
            Event::PointerMove { .. } => {
                if !self.hovered {
                    self.hovered = true;
                    cx.mark_needs_paint();
                }
                EventResult::Ignored
            }
            Event::PointerDown {
                button: PointerButton::Primary,
                ..
            } => {
                self.pressed = true;
                cx.mark_needs_paint();
                EventResult::Consumed
            }
            Event::PointerUp {
                button: PointerButton::Primary,
                ..
            } => {
                let was_pressed = self.pressed;
                self.pressed = false;
                cx.mark_needs_paint();
                if was_pressed {
                    if let Some(on_click) = self.on_click.as_mut() {
                        on_click();
                    }
                }
                EventResult::Consumed
            }
            _ => EventResult::Ignored,
        }
    }
}

pub struct ColumnWidget {
    pub gap: f32,
}

impl Widget for ColumnWidget {
    fn node_type(&self) -> NodeType {
        NodeType::Column
    }

    fn update_from_kind(&mut self, new_kind: &WidgetKind) -> DirtyFlags {
        let WidgetKind::Column { gap } = new_kind else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };
        if self.gap != *gap {
            self.gap = *gap;
            DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT
        } else {
            DirtyFlags::empty()
        }
    }

    fn paint(&self, _rect: Rect, _commands: &mut Vec<PaintCommand>) {}

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}

pub struct RowWidget {
    pub gap: f32,
}

impl Widget for RowWidget {
    fn node_type(&self) -> NodeType {
        NodeType::Row
    }

    fn update_from_kind(&mut self, new_kind: &WidgetKind) -> DirtyFlags {
        let WidgetKind::Row { gap } = new_kind else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };
        if self.gap != *gap {
            self.gap = *gap;
            DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT
        } else {
            DirtyFlags::empty()
        }
    }

    fn paint(&self, _rect: Rect, _commands: &mut Vec<PaintCommand>) {}

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
}

pub struct ContainerWidget {
    pub background: Color,
}

impl Widget for ContainerWidget {
    fn node_type(&self) -> NodeType {
        NodeType::Container
    }

    fn update_from_kind(&mut self, new_kind: &WidgetKind) -> DirtyFlags {
        let WidgetKind::Container { background } = new_kind else {
            return DirtyFlags::TREE | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
        };
        if self.background != *background {
            self.background = *background;
            DirtyFlags::PAINT
        } else {
            DirtyFlags::empty()
        }
    }

    fn paint(&self, rect: Rect, commands: &mut Vec<PaintCommand>) {
        if self.background.a > 0.0 {
            commands.push(PaintCommand::FillRect {
                rect,
                color: self.background,
            });
        }
    }

    fn handle_event(&mut self, _event: &Event, _cx: &mut EventContext<'_>) -> EventResult {
        EventResult::Ignored
    }
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
