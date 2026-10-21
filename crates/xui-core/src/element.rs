use crate::node::{Draft, NodeDef};
use crate::template::TemplateDesc;
use crate::widgets::WidgetType;
use crate::{
    ComponentRender,
    anchor::AnchorPlacement,
    fiber::{ErasedProps, Key},
    widgets::{DismissHandler, DismissReason, ErasedDraft, OverlayEntryOptions, OverlayScopeId},
};

pub type Component = ElementDesc;

/// One element of a tree, as a render describes it.
///
/// Four kinds, each reconciled its own way:
///
/// - a [`Template`](TemplateDesc) — the host widgets, in a structure fixed
///   where it is written, with the values that change. Every host is part of
///   a template: `xui!` and `view!` compile to one, and a draft written by
///   hand ([`Draft::into_element_desc`]) is a template of one host;
/// - a [`Component`](ComponentDesc) — a render function and its props;
/// - a [`Fragment`](FragmentDesc) — a list of elements standing in one place;
/// - a [`Portal`](PortalDesc) — elements mounted under the overlayer.
#[derive(Clone, Debug)]
pub enum ElementDesc {
    Template(TemplateDesc),
    Component(ComponentDesc),
    Fragment(FragmentDesc),
    Portal(PortalDesc),
}

/// A list of elements standing in one place among its siblings.
#[derive(Clone, Debug, Default)]
pub struct FragmentDesc {
    pub key: Option<Key>,
    pub children: Vec<ElementDesc>,
}

impl FragmentDesc {
    pub fn new(children: Vec<ElementDesc>) -> Self {
        Self {
            key: None,
            children,
        }
    }
}

/// An element with every template in it expanded into the hosts it holds:
/// what to inspect, in a test, of what a macro built. Not an element: it
/// is never rendered.
#[derive(Clone, Debug)]
pub enum Expanded {
    Host(ExpandedHost),
    Component(ComponentDesc),
    Portal(PortalDesc),
}

/// A host of an expanded template: its draft, and what is under it.
#[derive(Debug)]
pub struct ExpandedHost {
    pub(crate) draft: Box<dyn ErasedDraft>,
    pub children: Vec<Expanded>,
}

impl Clone for ExpandedHost {
    fn clone(&self) -> Self {
        Self {
            draft: self.draft.clone_box(),
            children: self.children.clone(),
        }
    }
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
    pub behavior: PortalBehavior,
    pub children: Vec<ElementDesc>,
}

/// What a Portal does beyond stacking. Each part goes to its own runtime
/// subsystem when the Portal commits: the anchor to anchoring, the dismiss
/// handler to the overlay model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PortalBehavior {
    /// Places the content's root against the host the Portal is written under.
    pub anchor: Option<AnchorPlacement>,
    pub on_dismiss: Option<DismissHandler>,
}

impl PortalDesc {
    pub fn new(children: Vec<ElementDesc>) -> Self {
        Self {
            key: None,
            scope: None,
            options: OverlayEntryOptions::default(),
            behavior: PortalBehavior::default(),
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

    /// Places the Portal's content next to the host it is written under -- the
    /// nearest host above the `portal(..)` in the component tree -- like a
    /// Flutter `CompositedTransformFollower` following its target.
    ///
    /// The content's root is taken out of flow and translated at paint and
    /// hit-test time, not in layout, so it stays attached through resizes and
    /// scrolling alike. `match_width` is the one part that reaches layout.
    /// A Portal directly under the root, or under another Portal, has no host
    /// to follow and is not moved.
    pub fn anchor(mut self, placement: AnchorPlacement) -> Self {
        self.behavior.anchor = Some(placement);
        self
    }

    /// Called when the Portal should close: a pointer went down outside its
    /// content, or Escape was pressed and nothing focused handled it.
    ///
    /// Only the topmost Portal with a handler is asked, and not through a modal
    /// Portal above it that has none. The Portal is not removed: the handler
    /// closes it by no longer rendering it.
    pub fn on_dismiss(mut self, handler: impl Fn(DismissReason) + 'static) -> Self {
        self.behavior.on_dismiss = Some(DismissHandler::new(handler));
        self
    }
}

impl ExpandedHost {
    /// Which host widget it is.
    pub fn node_type(&self) -> WidgetType {
        self.draft.node_type()
    }

    /// Its key among its siblings.
    pub fn key(&self) -> Option<&Key> {
        self.draft.key()
    }

    /// The draft it was built from, if it is a draft of `N`.
    pub fn draft<N: NodeDef>(&self) -> Option<&Draft<N>> {
        self.draft.as_any().downcast_ref::<Draft<N>>()
    }
}

impl Expanded {
    pub fn key(&self) -> Option<Key> {
        match self {
            Self::Host(host) => host.key().cloned(),
            Self::Component(component) => component.key,
            Self::Portal(portal) => portal.key,
        }
    }

    pub fn node_type(&self) -> Option<WidgetType> {
        match self {
            Self::Host(host) => Some(host.node_type()),
            _ => None,
        }
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
            Self::Template(template) => template.key,
            Self::Component(component) => component.key,
            Self::Fragment(fragment) => fragment.key,
            Self::Portal(portal) => portal.key,
        }
    }

    /// Which host widget the element's root is, when it is a template.
    pub fn node_type(&self) -> Option<WidgetType> {
        match self {
            Self::Template(template) => Some(template.root_type),
            _ => None,
        }
    }

    /// The element with every template in it expanded: the hosts it holds,
    /// fragments spliced into the list they stand in.
    pub fn expand(self) -> Vec<Expanded> {
        let mut out = Vec::new();
        self.expand_into(&mut out);
        out
    }

    /// [`expand`](Self::expand) of an element that stands for exactly one.
    pub fn expand_one(self) -> Expanded {
        let mut expanded = self.expand();
        assert_eq!(
            expanded.len(),
            1,
            "the element expands to {} elements",
            expanded.len()
        );
        expanded.pop().unwrap()
    }

    pub(crate) fn expand_into(self, out: &mut Vec<Expanded>) {
        match self {
            Self::Template(template) => out.push(template.expand()),
            Self::Component(component) => out.push(Expanded::Component(component)),
            Self::Fragment(fragment) => {
                for child in fragment.children {
                    child.expand_into(out);
                }
            }
            Self::Portal(portal) => out.push(Expanded::Portal(portal)),
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

impl From<FragmentDesc> for ElementDesc {
    fn from(fragment: FragmentDesc) -> Self {
        Self::Fragment(fragment)
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

impl From<TemplateDesc> for ElementDesc {
    fn from(template: TemplateDesc) -> Self {
        Self::Template(template)
    }
}

impl IntoChildren for TemplateDesc {
    fn append_children(self, output: &mut Vec<ElementDesc>) {
        output.push(self.into());
    }
}

pub fn portal(children: Vec<ElementDesc>) -> PortalDesc {
    PortalDesc::new(children)
}

/// Empty portal builder for use in the declarative `view!` DSL.
pub fn portal_view() -> PortalDesc {
    PortalDesc::new(Vec::new())
}
impl crate::dsl::IntoElement<crate::dsl::NoChildren> for PortalDesc {
    fn into_element(self, _: crate::dsl::NoChildren) -> ElementDesc {
        self.into()
    }
}
impl<T: IntoChildren> crate::dsl::IntoElement<crate::dsl::Content<T>> for PortalDesc {
    fn into_element(mut self, children: crate::dsl::Content<T>) -> ElementDesc {
        self.children = children.collect();
        self.into()
    }
}
impl crate::dsl::IntoElement<crate::dsl::Children> for PortalDesc {
    fn into_element(mut self, children: crate::dsl::Children) -> ElementDesc {
        self.children = children.collect();
        self.into()
    }
}
