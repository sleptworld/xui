use crate::{
    ComponentRender,
    anchor::AnchorPlacement,
    fiber::{ErasedProps, Key},
    event_system::callbacks::EventHandlers,
    event_system::interaction::HostInteraction,
    widgets::{
        DismissHandler, DismissReason, OverlayEntryOptions, OverlayScopeId, WidgetProto, Widgets,
    },
};
use std::rc::Rc;
use xui_interface::{Style, StyleMerge, TextContent, WidgetType, WidgetUpdateFlags};

pub type Component = ElementDesc;

#[derive(Clone, Debug)]
pub enum ElementDesc {
    Host(WidgetDesc),
    Component(ComponentDesc),
    Portal(PortalDesc),
    /// A subtree `xui!` / `view!` proved static: built once per call site and
    /// thread, and handed out as the same allocation on every render, so the
    /// reconciler can skip it by pointer.
    Hoisted(Rc<Hoisted>),
    /// A host element `xui!` / `view!` split at its call site into what never
    /// changes, built once ([`TemplateBase`]), and what may (the slots), so
    /// the reconciler compares and writes only the latter.
    Templated(TemplatedDesc),
}

/// A host element as its call site's [`TemplateBase`] plus this render's
/// values for the attributes that change. See `docs/template-slots.md` §12.
#[derive(Clone, Debug)]
pub struct TemplatedDesc {
    pub(crate) base: Rc<TemplateBase>,
    pub(crate) key: Option<Key>,
    pub(crate) slots: TemplateSlots,
    /// Boxed and absent when there are none, to keep this no larger than the
    /// other variants of `ElementDesc`.
    pub(crate) handlers: Option<Box<EventHandlers>>,
    pub children: Vec<ElementDesc>,
}

/// The part of a templated host that is the same on every render: the tag
/// with every static attribute applied. One per call site and thread.
#[derive(Debug)]
pub struct TemplateBase {
    proto: WidgetProto,
}

impl TemplateBase {
    pub fn new(proto: WidgetProto) -> Rc<Self> {
        Rc::new(Self { proto })
    }

    pub(crate) fn proto(&self) -> &WidgetProto {
        &self.proto
    }

    /// The interaction the static attributes set, with `handlers` from this
    /// render.
    pub(crate) fn interaction(
        &self,
        handlers: Option<Box<EventHandlers>>,
    ) -> Option<HostInteraction> {
        let interaction = HostInteraction {
            properties: self
                .proto
                .widgets()
                .interaction_properties()
                .cloned()
                .unwrap_or_default(),
            handlers: handlers.map(|handlers| *handlers).unwrap_or_default(),
        };
        (!interaction.is_empty()).then_some(interaction)
    }

    /// A widget built as the call site's builder chain would build it with
    /// `slots`, interaction left out (see [`Widgets::instantiate`]).
    pub(crate) fn build(&self, slots: &TemplateSlots) -> Widgets {
        let mut widget = self.proto.widgets().instantiate();
        slots.write(self, None, &mut widget);
        widget
    }
}

/// The values of a templated host's changing attributes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TemplateSlots {
    /// Every style attribute from the first changing one on, chained in
    /// source order onto an empty style. Merged over the base's style it gives
    /// what the builder chain would: the setters only assign fields.
    ///
    /// Boxed: a `Style` is a few kilobytes, and this struct travels inside
    /// every `ElementDesc`, work node and fiber, templated or not.
    pub(crate) style: Option<Box<Style>>,
    /// `text(..)`'s content, when it is not a constant.
    pub(crate) content: Option<TextContent>,
}

impl TemplateSlots {
    /// Writes the slots into `widget`, which holds `base`'s attributes and
    /// either nothing else (`prev` is `None`) or the slots `prev` wrote.
    /// Only what differs from `prev` is written; the result says what has to
    /// be redone.
    pub(crate) fn write(
        &self,
        base: &TemplateBase,
        prev: Option<&TemplateSlots>,
        widget: &mut Widgets,
    ) -> WidgetUpdateFlags {
        let mut flags = WidgetUpdateFlags::empty();
        if let Some(style) = &self.style
            && prev.is_none_or(|prev| prev.style.as_ref() != Some(style))
            && let Some(target) = widget.style_mut()
        {
            let mut next = base.proto.widgets().style().clone();
            style.merge_into(&mut next);
            *target = next;
            flags |= WidgetUpdateFlags::STYLE_TARGET;
        }
        if let Some(content) = &self.content
            && prev.is_none_or(|prev| prev.content.as_ref() != Some(content))
        {
            match widget {
                Widgets::Text(text) => {
                    text.props.text = content.clone();
                    flags |= WidgetUpdateFlags::LAYOUT_INPUT | WidgetUpdateFlags::PAINT_OUTPUT;
                }
                _ => debug_assert!(false, "only `text` has a content slot"),
            }
        }
        flags
    }
}

impl TemplatedDesc {
    pub fn key(&self) -> Option<Key> {
        self.key.or_else(|| self.base.proto.key())
    }

    /// The description the builder chain would have produced: the base with
    /// the slots written and the handlers installed.
    pub fn to_widget_desc(&self) -> WidgetDesc {
        let mut widget = self.base.build(&self.slots);
        if self.key.is_some() {
            widget.set_key(self.key);
        }
        if let Some(interaction) = self.base.interaction(self.handlers.clone()) {
            widget.set_interaction(interaction);
        }
        WidgetDesc::new(WidgetProto::new(widget), self.children.clone())
    }
}

/// The shared allocation behind [`ElementDesc::Hoisted`].
#[derive(Debug)]
pub struct Hoisted {
    element: ElementDesc,
    /// The host's `props_hash`, computed once rather than on every render.
    /// `None` when the element is not a host.
    props_hash: Option<u64>,
}

impl Hoisted {
    pub fn new(element: ElementDesc) -> Rc<Self> {
        match element {
            ElementDesc::Hoisted(hoisted) => hoisted,
            element => {
                let props_hash = match &element {
                    ElementDesc::Host(host) => Some(host.widget.props_hash()),
                    _ => None,
                };
                Rc::new(Self {
                    element,
                    props_hash,
                })
            }
        }
    }

    /// The element itself, never another `Hoisted`.
    pub fn element(&self) -> &ElementDesc {
        &self.element
    }

    pub(crate) fn props_hash(&self) -> Option<u64> {
        self.props_hash
    }
}

#[derive(Clone, Debug)]
pub struct WidgetDesc {
    pub widget: WidgetProto,
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

impl WidgetDesc {
    pub fn new(widget: WidgetProto, children: Vec<ElementDesc>) -> Self {
        Self { widget, children }
    }

    pub fn leaf(widget: WidgetProto) -> Self {
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
            Self::Hoisted(hoisted) => hoisted.element.key(),
            Self::Templated(templated) => templated.key(),
        }
    }

    /// The element as a plain host description, if it is a host: looking
    /// through a hoisted allocation and building a templated element out.
    pub fn to_host(&self) -> Option<WidgetDesc> {
        match self.resolved() {
            Self::Host(host) => Some(host.clone()),
            Self::Templated(templated) => Some(templated.to_widget_desc()),
            _ => None,
        }
    }

    /// The element itself, looking through a [`Hoisted`] allocation.
    pub fn resolved(&self) -> &ElementDesc {
        match self {
            Self::Hoisted(hoisted) => hoisted.element(),
            element => element,
        }
    }

    pub fn node_type(&self) -> Option<WidgetType> {
        match self {
            Self::Host(widget) => Some(widget.widget.node_type()),
            Self::Component(_) | Self::Portal(_) => None,
            Self::Hoisted(hoisted) => hoisted.element.node_type(),
            Self::Templated(templated) => Some(templated.base.proto.node_type()),
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
            ElementDesc::Hoisted(hoisted) => hoisted.element.clone().into(),
            ElementDesc::Templated(templated) => templated.to_widget_desc(),
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
            ElementDesc::Hoisted(hoisted) => hoisted.element.clone().into(),
            ElementDesc::Templated(_) => unreachable!(),
            ElementDesc::Portal(_) => unreachable!(),
        }
    }
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
