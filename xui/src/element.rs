use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::{
    ComponentRender,
    fiber::{ErasedProps, Key},
    widgets::{OverlayEntryOptions, OverlayScopeId, WidgetI},
};
use xui_interface::WidgetType;

pub type Component = ElementDesc;

#[derive(Clone, Debug)]
pub enum ElementDesc {
    Host(WidgetDesc),
    Component(ComponentDesc),
    Portal(PortalDesc),
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

/// Keeps children in the logical component tree while mounting their host
/// subtree below the runtime's root overlayer.
#[derive(Clone, Debug)]
pub struct PortalDesc {
    pub key: Option<Key>,
    pub scope: Option<OverlayScopeId>,
    pub options: OverlayEntryOptions,
    pub children: Vec<ElementDesc>,
}

impl PortalDesc {
    pub fn new(children: Vec<ElementDesc>) -> Self {
        Self {
            key: None,
            scope: None,
            options: OverlayEntryOptions::default(),
            children,
        }
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn scope(mut self, scope: OverlayScopeId) -> Self {
        self.scope = Some(scope);
        self
    }

    pub fn z_index(mut self, z_index: i32) -> Self {
        self.options.z_index = z_index;
        self
    }

    pub fn hit_test(mut self, hit_test: bool) -> Self {
        self.options.hit_test = hit_test;
        self
    }

    pub fn modal(mut self, modal: bool) -> Self {
        self.options.modal = modal;
        self
    }
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
            Self::Portal(portal) => portal.key,
        }
    }

    pub fn node_type(&self) -> Option<WidgetType> {
        match self {
            Self::Host(widget) => Some(widget.widget.node_type()),
            Self::Component(_) | Self::Portal(_) => None,
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
            Self::Portal(portal) => {
                let mut hasher = DefaultHasher::new();
                portal.key.hash(&mut hasher);
                portal.scope.hash(&mut hasher);
                portal.options.hash(&mut hasher);
                portal.children.hash(&mut hasher);
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

impl From<&ElementDesc> for ElementDesc {
    fn from(element: &ElementDesc) -> Self {
        element.clone()
    }
}

/// Converts either one element or a collection of elements into children.
///
/// The [`xui!`](crate::xui) macro uses this trait for braced child
/// expressions, allowing both `{child}` and `{children}` composition.
pub trait IntoChildren {
    fn append_children(self, output: &mut Vec<ElementDesc>);
}

impl IntoChildren for ElementDesc {
    fn append_children(self, output: &mut Vec<ElementDesc>) {
        output.push(self);
    }
}

impl IntoChildren for WidgetDesc {
    fn append_children(self, output: &mut Vec<ElementDesc>) {
        output.push(self.into());
    }
}

impl IntoChildren for ComponentDesc {
    fn append_children(self, output: &mut Vec<ElementDesc>) {
        output.push(self.into());
    }
}

impl IntoChildren for PortalDesc {
    fn append_children(self, output: &mut Vec<ElementDesc>) {
        output.push(self.into());
    }
}

impl<I, T> IntoChildren for I
where
    I: IntoIterator<Item = T>,
    T: Into<ElementDesc>,
{
    fn append_children(self, output: &mut Vec<ElementDesc>) {
        output.extend(self.into_iter().map(Into::into));
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

impl From<PortalDesc> for ElementDesc {
    fn from(portal: PortalDesc) -> Self {
        Self::Portal(portal)
    }
}

pub fn portal(children: Vec<ElementDesc>) -> PortalDesc {
    PortalDesc::new(children)
}

impl Into<WidgetDesc> for ElementDesc {
    fn into(self) -> WidgetDesc {
        match self {
            ElementDesc::Host(widget) => widget,
            ElementDesc::Component(_) => {
                unreachable!()
            }
            ElementDesc::Portal(_) => unreachable!(),
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
            ElementDesc::Portal(_) => unreachable!(),
        }
    }
}
