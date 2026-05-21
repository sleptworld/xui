use std::hash::{Hash, Hasher};
pub use xui_interface::Key;
use xui_interface::{
    Color, EdgeInsets, Event, EventContext, EventHandlers, EventResult, InputKey, Size,
};

macro_rules! event_handler_methods {
    () => {
        pub fn on_event(
            mut self,
            handler: impl for<'a> FnMut(&Event, &mut EventContext<'a>) -> EventResult + 'static,
        ) -> Self {
            self.event_handlers.on_event = Some(Box::new(handler));
            self
        }

        pub fn on_click(
            mut self,
            handler: impl for<'a> FnMut(&mut EventContext<'a>) -> EventResult + 'static,
        ) -> Self {
            self.event_handlers.on_click = Some(Box::new(handler));
            self
        }

        pub fn on_hover_change(
            mut self,
            handler: impl for<'a> FnMut(bool, &mut EventContext<'a>) -> EventResult + 'static,
        ) -> Self {
            self.event_handlers.on_hover_change = Some(Box::new(handler));
            self
        }

        pub fn on_pointer_down(
            mut self,
            handler: impl for<'a> FnMut(&mut EventContext<'a>) -> EventResult + 'static,
        ) -> Self {
            self.event_handlers.on_pointer_down = Some(Box::new(handler));
            self
        }

        pub fn on_pointer_up(
            mut self,
            handler: impl for<'a> FnMut(&mut EventContext<'a>) -> EventResult + 'static,
        ) -> Self {
            self.event_handlers.on_pointer_up = Some(Box::new(handler));
            self
        }

        pub fn on_pointer_move(
            mut self,
            handler: impl for<'a> FnMut(&mut EventContext<'a>) -> EventResult + 'static,
        ) -> Self {
            self.event_handlers.on_pointer_move = Some(Box::new(handler));
            self
        }

        pub fn on_key_down(
            mut self,
            handler: impl for<'a> FnMut(&InputKey, &mut EventContext<'a>) -> EventResult + 'static,
        ) -> Self {
            self.event_handlers.on_key_down = Some(Box::new(handler));
            self
        }

        pub fn on_key_up(
            mut self,
            handler: impl for<'a> FnMut(&InputKey, &mut EventContext<'a>) -> EventResult + 'static,
        ) -> Self {
            self.event_handlers.on_key_up = Some(Box::new(handler));
            self
        }
    };
}

#[derive(Debug)]
pub struct Label {
    pub key: Option<Key>,
    pub text: String,
    pub color: Color,
    pub font_size: f32,
    pub event_handlers: EventHandlers,
}

impl Label {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            key: None,
            color: Color::BLACK,
            font_size: 14.0,
            event_handlers: EventHandlers::default(),
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

    event_handler_methods!();
}

pub struct Button {
    pub key: Option<Key>,
    pub text: String,
    pub event_handlers: EventHandlers,
}

impl Button {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            key: None,
            event_handlers: EventHandlers::default(),
        }
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    event_handler_methods!();
}

pub struct Column<Child = ()> {
    pub key: Option<Key>,
    pub children: Vec<Child>,
    pub gap: f32,
    pub event_handlers: EventHandlers,
}

impl<Child> Column<Child> {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
            key: None,
            gap: 0.0,
            event_handlers: EventHandlers::default(),
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

    event_handler_methods!();
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
    pub event_handlers: EventHandlers,
}

impl<Child> Row<Child> {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
            key: None,
            gap: 0.0,
            event_handlers: EventHandlers::default(),
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

    event_handler_methods!();
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
    pub event_handlers: EventHandlers,
}

impl<Child> Container<Child> {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
            key: None,
            size: None,
            padding: EdgeInsets::ZERO,
            background: Color::TRANSPARENT,
            event_handlers: EventHandlers::default(),
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

    event_handler_methods!();
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
