use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::{
    ComponentType,
    fiber::{ErasedProps, Key},
    widgets::WidgetI,
};
use xui_interface::WidgetType;

#[derive(Debug, Clone)]
pub enum ElementDesc {
    Host(WidgetDesc),
    Component(ComponentDesc),
}

#[derive(Debug, Clone)]
pub struct WidgetDesc {
    pub widget: WidgetI,
    pub children: Vec<ElementDesc>,
}

#[derive(Debug, Clone)]
pub struct ComponentDesc {
    pub key: Option<Key>,
    pub component_type: ComponentType,
    pub props: ErasedProps,
    pub props_hash: u64,
    pub children: Vec<ElementDesc>,
}

impl WidgetDesc {
    pub fn new(widget: WidgetI, children: Vec<ElementDesc>) -> Self {
        Self { widget, children }
    }

    pub fn leaf(widget: WidgetI) -> Self {
        Self::new(widget, Vec::new())
    }

    pub fn child(mut self, child: impl Into<ElementDesc>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn with_children(mut self, children: Vec<ElementDesc>) -> Self {
        self.children = children;
        self
    }
}

impl ComponentDesc {
    pub fn new(
        component_type: ComponentType,
        props: ErasedProps,
        props_hash: u64,
        children: Vec<ElementDesc>,
    ) -> Self {
        Self {
            key: None,
            component_type,
            props,
            props_hash,
            children,
        }
    }

    pub fn child(mut self, child: impl Into<ElementDesc>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn with_children(mut self, children: Vec<ElementDesc>) -> Self {
        self.children = children;
        self
    }
}

impl ElementDesc {
    pub fn key(&self) -> Option<Key> {
        match self {
            Self::Host(widget) => widget.widget.key(),
            Self::Component(component) => component.key.clone(),
        }
    }

    pub fn node_type(&self) -> Option<WidgetType> {
        match self {
            Self::Host(widget) => Some(widget.widget.node_type()),
            Self::Component(_) => None,
        }
    }

    pub fn props_hash(&self) -> u64 {
        match self {
            Self::Host(widget) => widget.widget.props_hash(),
            Self::Component(component) => {
                let mut hasher = DefaultHasher::new();
                component.key.hash(&mut hasher);
                component.component_type.hash(&mut hasher);
                component.props_hash.hash(&mut hasher);
                hasher.finish()
            }
        }
    }
}

impl Into<ElementDesc> for WidgetDesc {
    fn into(self) -> ElementDesc {
        ElementDesc::Host(self)
    }
}

impl Into<ElementDesc> for ComponentDesc {
    fn into(self) -> ElementDesc {
        ElementDesc::Component(self)
    }
}

impl Into<WidgetDesc> for ElementDesc {
    fn into(self) -> WidgetDesc {
        match self {
            ElementDesc::Host(widget) => widget,
            ElementDesc::Component(_) => {
                unreachable!()
            }
        }
    }
}

impl Into<ComponentDesc> for ElementDesc {
    fn into(self) -> ComponentDesc {
        match self {
            ElementDesc::Host(_) => {
                unreachable!()
            }
            ElementDesc::Component(component) => component,
        }
    }
}
