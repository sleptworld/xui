use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::{
    ComponentRender,
    fiber::{ErasedProps, Key},
    widgets::WidgetI,
};
use xui_interface::WidgetType;

#[derive(Clone, Debug)]
pub enum ElementDesc {
    Host(WidgetDesc),
    Component(ComponentDesc),
}

#[derive(Clone, Debug)]
pub struct WidgetDesc {
    pub widget: WidgetI,
    pub children: Vec<ElementDesc>,
}

#[derive(Clone, Debug)]
pub struct ComponentDesc {
    pub key: Option<Key>,
    pub render: ComponentRender,
    pub props: Option<ErasedProps>,
    pub props_hash: u64,
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
    pub fn new(render: ComponentRender, props: Option<ErasedProps>, props_hash: u64) -> Self {
        Self {
            key: None,
            render,
            props,
            props_hash,
        }
    }
}

impl ElementDesc {
    pub fn key(&self) -> Option<Key> {
        match self {
            Self::Host(widget) => widget.widget.key(),
            Self::Component(component) => component.key,
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
                component.render.hash(&mut hasher);
                component.props_hash.hash(&mut hasher);
                hasher.finish()
            }
        }
    }
}

impl Hash for ElementDesc {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.props_hash().hash(state);
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
