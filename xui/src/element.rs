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
    pub fn new(render: ComponentRender, props: Option<ErasedProps>) -> Self {
        Self {
            key: None,
            render,
            props,
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

impl From<WidgetDesc> for ElementDesc {
    fn from(val: WidgetDesc) -> Self {
        ElementDesc::Host(val)
    }
}

impl From<ComponentDesc> for ElementDesc {
    fn from(val: ComponentDesc) -> Self {
        ElementDesc::Component(val)
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

impl From<ElementDesc> for WidgetDesc {
    fn from(val: ElementDesc) -> Self {
        match val {
            ElementDesc::Host(widget) => widget,
            ElementDesc::Component(_) => {
                unreachable!()
            }
            ElementDesc::Portal(_) => unreachable!(),
        }
    }
}

impl From<ElementDesc> for ComponentDesc {
    fn from(val: ElementDesc) -> Self {
        match val {
            ElementDesc::Host(_) => {
                unreachable!()
            }
            ElementDesc::Component(component) => component,
            ElementDesc::Portal(_) => unreachable!(),
        }
    }
}
