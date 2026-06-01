use std::cell::RefCell;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use xui::prelude::*;

#[derive(Clone)]
pub struct ButtonClickHandler {
    handler: Rc<RefCell<ClickEventHandler>>,
}

impl ButtonClickHandler {
    pub fn new(
        handler: impl for<'a> FnMut(&mut EventContext<'a>) -> EventResult + 'static,
    ) -> Self {
        Self {
            handler: Rc::new(RefCell::new(Box::new(handler))),
        }
    }

    fn call(&self, cx: &mut EventContext<'_>) -> EventResult {
        (self.handler.borrow_mut())(cx)
    }
}

impl fmt::Debug for ButtonClickHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ButtonClickHandler").finish_non_exhaustive()
    }
}

impl Hash for ButtonClickHandler {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Rc::as_ptr(&self.handler).hash(state);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ButtonVariant {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ButtonSize {
    Sm,
    Md,
    Lg,
}

pub struct ButtonProps {
    pub text: TextContent,
    pub children: Vec<ElementDesc>,
    pub variant: ButtonVariant,
    pub size: ButtonSize,
    pub disabled: bool,
    pub style: Style,
    pub hover_style: Style,
    pub pressed_style: Style,
    pub disabled_style: Style,
    pub on_click: Option<ButtonClickHandler>,
}

impl ButtonProps {
    pub fn new(text: impl Into<TextContent>) -> Self {
        Self {
            text: text.into(),
            children: Vec::new(),
            variant: ButtonVariant::Primary,
            size: ButtonSize::Md,
            disabled: false,
            style: Style::new(),
            hover_style: Style::new(),
            pressed_style: Style::new(),
            disabled_style: Style::new(),
            on_click: None,
        }
    }

    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn child(mut self, child: impl Into<ElementDesc>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn hover_style(mut self, style: Style) -> Self {
        self.hover_style = style;
        self
    }

    pub fn pressed_style(mut self, style: Style) -> Self {
        self.pressed_style = style;
        self
    }

    pub fn disabled_style(mut self, style: Style) -> Self {
        self.disabled_style = style;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl for<'a> FnMut(&mut EventContext<'a>) -> EventResult + 'static,
    ) -> Self {
        self.on_click = Some(ButtonClickHandler::new(handler));
        self
    }

    pub fn on_click_handler(mut self, handler: ButtonClickHandler) -> Self {
        self.on_click = Some(handler);
        self
    }
}

impl fmt::Debug for ButtonProps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ButtonProps")
            .field("text", &self.text)
            .field("children", &self.children.len())
            .field("variant", &self.variant)
            .field("size", &self.size)
            .field("disabled", &self.disabled)
            .field("style", &self.style)
            .field("hover_style", &self.hover_style)
            .field("pressed_style", &self.pressed_style)
            .field("disabled_style", &self.disabled_style)
            .field("on_click", &self.on_click)
            .finish()
    }
}

impl Default for ButtonProps {
    fn default() -> Self {
        Self::new("")
    }
}

impl WithChildren for ButtonProps {
    fn with_children(mut self, children: Vec<ElementDesc>) -> Self {
        self.children = children;
        self
    }
}

impl Hash for ButtonProps {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
        self.children.len().hash(state);
        for child in &self.children {
            child.props_hash().hash(state);
        }
        self.variant.hash(state);
        self.size.hash(state);
        self.disabled.hash(state);
        self.style.hash(state);
        self.hover_style.hash(state);
        self.pressed_style.hash(state);
        self.disabled_style.hash(state);
        self.on_click.hash(state);
    }
}

component_fn! {
    pub fn pbutton(props: &ButtonProps) {
        let mut button = button(if props.children.is_empty() {
                props.text.clone()
            } else {
                TextContent::default()
            })
            .disabled(props.disabled)
            .style(merged_style(button_style(props.variant, props.size), &props.style))
            .hover_style(merged_style(hover_style(props.variant), &props.hover_style))
            .pressed_style(merged_style(pressed_style(props.variant), &props.pressed_style))
            .disabled_style(merged_style(disabled_style(), &props.disabled_style));

        if !props.disabled {
            if let Some(handler) = props.on_click.clone() {
                button = button.on_click(move |cx| handler.call(cx));
            }
        }

        button.into_element_desc(props.children.clone())
    }
}

pub fn pbutton(props: ButtonProps) -> ComponentDesc {
    component(pbutton_component_type()).props(props)
}

fn merged_style(mut base: Style, override_style: &Style) -> Style {
    base.merge(override_style);
    base
}

fn button_style(variant: ButtonVariant, size: ButtonSize) -> Style {
    merged_style(variant_style(variant), &size_style(size))
}

fn variant_style(variant: ButtonVariant) -> Style {
    match variant {
        ButtonVariant::Primary => Style::new()
            .background(ColorToken::Primary)
            .border_color(ColorToken::Primary)
            .color(ColorToken::InverseText),
        ButtonVariant::Secondary => Style::new()
            .background(ColorToken::Surface)
            .border_color(ColorToken::Border)
            .color(ColorToken::Text),
        ButtonVariant::Ghost => Style::new()
            .background(Color::TRANSPARENT)
            .border_color(Color::TRANSPARENT)
            .color(ColorToken::Text),
        ButtonVariant::Danger => Style::new()
            .background(Color::rgb(0.84, 0.14, 0.14))
            .border_color(Color::rgb(0.84, 0.14, 0.14))
            .color(Color::WHITE),
    }
    .border_width(1.0)
    .border_radius(6.0)
}

fn size_style(size: ButtonSize) -> Style {
    let (padding, font_size) = match size {
        ButtonSize::Sm => (
            EdgeInsets {
                left: 10.0,
                right: 10.0,
                top: 4.0,
                bottom: 4.0,
            },
            12.0,
        ),
        ButtonSize::Md => (
            EdgeInsets {
                left: 12.0,
                right: 12.0,
                top: 6.0,
                bottom: 6.0,
            },
            14.0,
        ),
        ButtonSize::Lg => (
            EdgeInsets {
                left: 16.0,
                right: 16.0,
                top: 8.0,
                bottom: 8.0,
            },
            16.0,
        ),
    };

    Style::new().padding(padding).font_size(font_size)
}

fn hover_style(variant: ButtonVariant) -> Style {
    match variant {
        ButtonVariant::Primary => Style::new().background(Color::BLUE_500),
        ButtonVariant::Secondary => Style::new().background(ColorToken::MutedSurface),
        ButtonVariant::Ghost => Style::new().background(ColorToken::MutedSurface),
        ButtonVariant::Danger => Style::new().background(Color::rgb(0.70, 0.09, 0.09)),
    }
}

fn pressed_style(variant: ButtonVariant) -> Style {
    match variant {
        ButtonVariant::Primary => Style::new()
            .background(Color::BLACK)
            .border_color(Color::BLACK)
            .color(Color::WHITE),
        ButtonVariant::Secondary | ButtonVariant::Ghost => {
            Style::new().background(ColorToken::Border)
        }
        ButtonVariant::Danger => Style::new()
            .background(Color::rgb(0.48, 0.05, 0.05))
            .border_color(Color::rgb(0.48, 0.05, 0.05))
            .color(Color::WHITE),
    }
}

fn disabled_style() -> Style {
    Style::new()
        .background(Color::GRAY_100)
        .border_color(Color::GRAY_300)
        .color(Color::GRAY_300)
}

#[allow(dead_code)]
fn default_button_style() -> Style {
    button_style(ButtonVariant::Primary, ButtonSize::Md)
}

#[allow(dead_code)]
fn default_hover_style() -> Style {
    hover_style(ButtonVariant::Primary)
}

#[allow(dead_code)]
fn default_pressed_style() -> Style {
    pressed_style(ButtonVariant::Primary)
}

#[allow(dead_code)]
fn default_disabled_style() -> Style {
    disabled_style()
}
