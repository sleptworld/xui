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
