use std::any::Any;
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use taffy::prelude as tf;
pub use xui_interface::{EventHandlers, Widget, WidgetType};
use xui_interface::{TextContent, TextMeasurer, widget};

use crate::core::{EdgeInsets, Size};
use crate::fiber::{ComponentType, ErasedProps, Key};
use crate::state::HookContext;
use crate::style::{ComputedStyle, DisplayStyle, FlexDirectionStyle, Style, Theme};

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
mod style_scope;
mod text;

pub use button::ButtonWidget;
pub use column::ColumnWidget;
pub use container::ContainerWidget;
pub use label::LabelWidget;
pub(crate) use label::apply_text_style;
pub use row::RowWidget;
pub use style_scope::StyleScopeWidget;
pub use text::TextWidget;

pub type ComponentRender = Rc<RefCell<dyn for<'a> FnMut(&mut HookContext<'a>) -> Element>>;

#[derive(Clone, Debug)]
pub struct WidgetRef {
    widget: Rc<RefCell<Box<dyn LayoutStyledWidget>>>,
}

impl WidgetRef {
    pub fn new(widget: impl LayoutStyledWidget + 'static) -> Self {
        Self::from(Box::new(widget) as Box<dyn LayoutStyledWidget>)
    }

    pub fn with<R>(&self, f: impl FnOnce(&dyn Widget) -> R) -> R {
        let widget = self.widget.borrow();
        f(&**widget)
    }

    pub fn with_mut<R>(&self, f: impl FnOnce(&mut dyn Widget) -> R) -> R {
        let mut widget = self.widget.borrow_mut();
        f(&mut **widget)
    }

    pub fn layout_with<R>(&self, f: impl FnOnce(&dyn LayoutStyledWidget) -> R) -> R {
        let widget = self.widget.borrow();
        f(&**widget)
    }
}

impl From<Box<dyn LayoutStyledWidget>> for WidgetRef {
    fn from(widget: Box<dyn LayoutStyledWidget>) -> Self {
        Self {
            widget: Rc::new(RefCell::new(widget)),
        }
    }
}

#[derive(Clone)]
pub struct HostElement {
    widget: WidgetRef,
    children: Vec<Element>,
}

#[derive(Clone)]
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
    pub fn new(widget: impl LayoutStyledWidget + 'static, children: Vec<Element>) -> Self {
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

    pub fn style<T: TextMeasurer>(&self, measurer: &mut T) -> tf::Style {
        self.widget
            .layout_with(|widget| style_for_widget(widget, measurer))
    }

    pub fn computed_style(&self, parent: &ComputedStyle, theme: &Theme) -> ComputedStyle {
        self.widget
            .with(|widget| computed_style_for_widget(widget, parent, theme))
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

    pub fn style<T: TextMeasurer>(&self, measurer: &mut T) -> tf::Style {
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

#[derive(Clone)]
pub struct ComponentElement {
    pub key: Option<Key>,
    pub render: ComponentType,
    pub props: ErasedProps,
    pub props_hash: u64,
}

pub trait WithChildren {
    fn with_children(self, children: Vec<Element>) -> Self;
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

pub fn component(render: ComponentType) -> ComponentElement {
    ComponentElement::new(render)
}

impl From<LabelWidget> for Element {
    fn from(value: LabelWidget) -> Self {
        Self::Host(HostElement::new(value, Vec::new()))
    }
}

impl From<TextWidget> for Element {
    fn from(value: TextWidget) -> Self {
        Self::Host(HostElement::new(value, Vec::new()))
    }
}

impl From<ButtonWidget> for Element {
    fn from(mut value: ButtonWidget) -> Self {
        let children = std::mem::take(&mut value.children);
        Self::Host(HostElement::new(value, children))
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

impl From<StyleScopeWidget> for Element {
    fn from(mut value: StyleScopeWidget) -> Self {
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

pub fn root_widget() -> Box<dyn LayoutStyledWidget> {
    Box::new(root::RootWidget::default())
}

pub fn computed_style_for_widget(
    widget: &dyn Widget,
    parent: &ComputedStyle,
    theme: &Theme,
) -> ComputedStyle {
    let mut computed = parent.inherited_from(theme);
    if let Some(scope) = widget.style_scope() {
        computed.apply(parent, scope, theme);
    }
    computed.apply(parent, &widget.default_style(), theme);
    computed.apply(parent, widget.style(), theme);
    computed.apply(parent, &widget.state_style(widget.state()), theme);
    computed
}

pub fn style_for_widget<T: TextMeasurer>(
    widget: &dyn LayoutStyledWidget,
    measurer: &mut T,
) -> tf::Style {
    let theme = Theme::default();
    let computed = computed_style_for_widget(widget, &ComputedStyle::initial(&theme), &theme);
    taffy_style_for_widget(widget, &computed, measurer)
}

#[inline]
pub fn taffy_style_for_widget<T: TextMeasurer>(
    widget: &dyn LayoutStyledWidget,
    computed: &ComputedStyle,
    measurer: &mut T,
) -> tf::Style {
    return widget.layout_style(computed, measurer);
}

pub fn computed_layout_style(computed: &ComputedStyle) -> tf::Style {
    let layout = computed.layout;
    let mut style = tf::Style {
        display: match layout.display {
            DisplayStyle::Flex => tf::Display::Flex,
            DisplayStyle::None => tf::Display::None,
        },
        flex_direction: match layout.flex_direction {
            FlexDirectionStyle::Row => tf::FlexDirection::Row,
            FlexDirectionStyle::Column => tf::FlexDirection::Column,
        },
        gap: tf::Size {
            width: length_percentage(layout.gap),
            height: length_percentage(layout.gap),
        },
        margin: edge_insets_auto(layout.margin),
        padding: edge_insets(layout.padding),
        ..Default::default()
    };

    if let Some(size) = layout.size {
        style.size = tf::Size {
            width: dimension(size.width),
            height: dimension(size.height),
        };
    }
    if let Some(size) = layout.min_size {
        style.min_size = tf::Size {
            width: dimension(size.width),
            height: dimension(size.height),
        };
    }
    if let Some(size) = layout.max_size {
        style.max_size = tf::Size {
            width: dimension(size.width),
            height: dimension(size.height),
        };
    }

    style
}

pub(super) fn fixed_size_style(size: Size) -> tf::Style {
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

fn edge_insets_auto(value: EdgeInsets) -> tf::Rect<tf::LengthPercentageAuto> {
    tf::Rect {
        left: tf::LengthPercentageAuto::length(value.left),
        right: tf::LengthPercentageAuto::length(value.right),
        top: tf::LengthPercentageAuto::length(value.top),
        bottom: tf::LengthPercentageAuto::length(value.bottom),
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

pub trait LayoutStyledWidget: Widget {
    fn layout_style(&self, computed: &ComputedStyle, measurer: &mut dyn TextMeasurer) -> tf::Style;
}
