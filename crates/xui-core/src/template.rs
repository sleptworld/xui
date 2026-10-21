//! Static templates: what `xui!` and `view!` compile an element tree to.
//!
//! An invocation is split at compile time into a structure that never
//! changes and the values that do:
//!
//! - a [`TemplateMeta`], one `static` per call site: the host nodes as
//!   written, their parents, which of them have dynamic attributes, and the
//!   *slots* between them;
//! - the inputs, evaluated where the macro is written, every render: each
//!   dynamic attribute already converted to its property's value, each
//!   component already an element, each dynamic child list already a list.
//!
//! The host nodes of a template are its own: the runtime mounts them once,
//! all together, and on a later render with the same template rebuilds only
//! the nodes that have a dynamic attribute. The static ones are not visited.
//! What can change shape lives in slots, which are ordinary elements the
//! reconciler diffs:
//!
//! - a **component** node is a slot of its own (its children are a value: the
//!   `children` prop), and
//! - a host whose child list holds anything but elements — a `{expr}`, an
//!   `if`, a `for` — has that whole list as one **fragment** slot, each
//!   element in it a template of its own.
//!
//! Host tags are known by name: the set of host widgets is closed, and the
//! generated code asserts that a tag treated as a host is one.
//!
//! [`TemplateDesc::expand`] turns a template into the plain element tree it
//! stands for; the runtime's handling of a template is equivalent to
//! reconciling that tree.

use std::any::TypeId;

use crate::element::{ElementDesc, WidgetDesc};
use crate::event_system::{EventContext, Flow, Handler, HandlerValue};
use crate::fiber::Key;
use crate::node::{Draft, NodeDef};
use crate::property::{IntoProp, Property, PropertyMarker};
use crate::widgets::{ErasedDraft, WidgetType, WidgetVariant};

/// The static structure of one `xui!` / `view!` call site.
#[derive(Debug)]
pub struct TemplateMeta {
    /// Where it was written, for diagnostics.
    pub location: &'static str,
    /// Every node, in preorder: a parent comes before its children, and
    /// siblings appear in the order written.
    pub nodes: &'static [TemplateNode],
}

/// One node of a [`TemplateMeta`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateNode {
    pub kind: TemplateNodeKind,
    /// Its parent's index; `None` only for the root, which is a host.
    pub parent: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateNodeKind {
    /// A host node the template owns. `dynamic`: it has an attribute that is
    /// not a constant, so a later render rebuilds it.
    Host { tag: &'static str, dynamic: bool },
    /// A component element, whose place the reconciler keeps.
    Component,
    /// The whole child list of its parent, reconciled as a list.
    Fragment,
}

impl TemplateMeta {
    /// The slots — component and fragment nodes — in order.
    pub(crate) fn slots(&self) -> impl Iterator<Item = (usize, &TemplateNode)> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| !matches!(node.kind, TemplateNodeKind::Host { .. }))
    }

    /// The nodes following `index` under the same parent, in order.
    pub(crate) fn following_siblings(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        let parent = self.nodes[index].parent;
        (index + 1..self.nodes.len()).filter(move |&other| self.nodes[other].parent == parent)
    }
}

/// What tells two templates apart: the call site, and the types of what it
/// was given, which differ between instantiations of a generic function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TemplateIdentity {
    meta: *const TemplateMeta,
    inputs: TypeId,
}

/// One render of a template: its call site, its key, and its inputs.
pub struct TemplateDesc {
    pub(crate) meta: &'static TemplateMeta,
    pub(crate) identity: TemplateIdentity,
    pub key: Option<Key>,
    /// Which host widget the root is.
    pub(crate) root_type: WidgetType,
    instance: Box<dyn ErasedTemplate>,
}

impl std::fmt::Debug for TemplateDesc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TemplateDesc")
            .field("location", &self.meta.location)
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl Clone for TemplateDesc {
    fn clone(&self) -> Self {
        Self {
            meta: self.meta,
            identity: self.identity,
            key: self.key.clone(),
            root_type: self.root_type,
            instance: self.instance.clone_box(),
        }
    }
}

trait ErasedTemplate {
    fn build(self: Box<Self>, out: &mut TemplateBuild);
    fn clone_box(&self) -> Box<dyn ErasedTemplate>;
}

struct Instance<I> {
    inputs: I,
    build: fn(I, &mut TemplateBuild),
}

impl<I: Clone + 'static> ErasedTemplate for Instance<I> {
    fn build(self: Box<Self>, out: &mut TemplateBuild) {
        (self.build)(self.inputs, out);
    }

    fn clone_box(&self) -> Box<dyn ErasedTemplate> {
        Box::new(Instance {
            inputs: self.inputs.clone(),
            build: self.build,
        })
    }
}

impl TemplateDesc {
    /// A render of the template at `meta` with `inputs`, which `build`
    /// turns into the template's drafts and slots. Generated by the element
    /// macros.
    #[doc(hidden)]
    pub fn new<I: Clone + 'static>(
        meta: &'static TemplateMeta,
        key: Option<Key>,
        root_type: WidgetType,
        inputs: I,
        build: fn(I, &mut TemplateBuild),
    ) -> Self {
        Self {
            meta,
            identity: TemplateIdentity {
                meta,
                inputs: TypeId::of::<I>(),
            },
            key,
            root_type,
            instance: Box::new(Instance { inputs, build }),
        }
    }

    /// Where the template was written.
    pub fn location(&self) -> &'static str {
        self.meta.location
    }

    /// The structure of the template.
    pub fn meta(&self) -> &'static TemplateMeta {
        self.meta
    }

    /// Runs the template: every host's draft when `mode` mounts or
    /// expands, only the dynamic ones' when it updates, and every slot.
    pub(crate) fn build(self, mode: BuildMode) -> TemplateBuild {
        let mut out = TemplateBuild {
            meta: self.meta,
            mode,
            hosts: (0..self.meta.nodes.len()).map(|_| None).collect(),
            slots: Vec::new(),
        };
        self.instance.build(&mut out);
        out
    }

    /// The plain element tree this template stands for: each host a
    /// [`WidgetDesc`] with its draft, each slot in its place.
    pub fn expand(self) -> ElementDesc {
        let key = self.key.clone();
        let meta = self.meta;
        let mut built = self.build(BuildMode::Mount);
        let mut slots = std::mem::take(&mut built.slots).into_iter();
        let mut elements: Vec<Option<ElementDesc>> = (0..meta.nodes.len()).map(|_| None).collect();
        let mut fragments: Vec<Option<Vec<ElementDesc>>> =
            (0..meta.nodes.len()).map(|_| None).collect();
        // Slots arrive in node order.
        for (index, node) in meta.nodes.iter().enumerate() {
            match node.kind {
                TemplateNodeKind::Component => {
                    let Some(TemplateSlot::Component(element)) = slots.next() else {
                        panic!("template {} lost a component slot", meta.location);
                    };
                    elements[index] = Some(element);
                }
                TemplateNodeKind::Fragment => {
                    let Some(TemplateSlot::Fragment(children)) = slots.next() else {
                        panic!("template {} lost a fragment slot", meta.location);
                    };
                    fragments[index] = Some(children);
                }
                TemplateNodeKind::Host { .. } => {}
            }
        }
        // Children come after their parent, so assembling back to front
        // finds every child finished.
        for index in (0..meta.nodes.len()).rev() {
            let TemplateNodeKind::Host { .. } = meta.nodes[index].kind else {
                continue;
            };
            let mut children = Vec::new();
            for child in (index + 1..meta.nodes.len()).filter(|&c| meta.nodes[c].parent == Some(index as u16)) {
                match meta.nodes[child].kind {
                    TemplateNodeKind::Fragment => {
                        children.extend(fragments[child].take().expect("fragment built once"))
                    }
                    _ => children.push(elements[child].take().expect("child built once")),
                }
            }
            let draft = built.hosts[index]
                .take()
                .unwrap_or_else(|| panic!("template {} built no draft for a host", meta.location));
            elements[index] = Some(WidgetDesc::new(draft, children).into());
        }
        let mut root = elements[0].take().expect("a template has a root");
        if let (Some(key), ElementDesc::Host(host)) = (key, &mut root) {
            host.set_key(key);
        }
        root
    }
}

/// What a template build is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BuildMode {
    /// The first render, or an expansion: every host.
    Mount,
    /// A later render of a mounted template: only the dynamic hosts.
    Update,
}

/// A slot's content, one render's worth.
pub(crate) enum TemplateSlot {
    Component(ElementDesc),
    Fragment(Vec<ElementDesc>),
}

/// What a template's build function fills in. Public only because generated
/// code names it.
#[doc(hidden)]
pub struct TemplateBuild {
    meta: &'static TemplateMeta,
    mode: BuildMode,
    pub(crate) hosts: Vec<Option<Box<dyn ErasedDraft>>>,
    pub(crate) slots: Vec<TemplateSlot>,
}

impl TemplateBuild {
    /// Whether host `index` is built this time: always on mount, and on an
    /// update only when it has a dynamic attribute.
    pub fn wants(&self, index: u16) -> bool {
        match self.meta.nodes[index as usize].kind {
            TemplateNodeKind::Host { dynamic, .. } => self.mode == BuildMode::Mount || dynamic,
            _ => false,
        }
    }

    /// Host `index`'s draft.
    pub fn host<N>(&mut self, index: u16, draft: Draft<N>)
    where
        N: WidgetVariant,
        N::Props: Clone,
        N::Behavior: Clone,
    {
        self.hosts[index as usize] = Some(Box::new(draft));
    }

    /// The next slot: a component element.
    pub fn component(&mut self, element: ElementDesc) {
        self.slots.push(TemplateSlot::Component(element));
    }

    /// The next slot: a child list.
    pub fn fragment(&mut self, children: Vec<ElementDesc>) {
        self.slots.push(TemplateSlot::Fragment(children));
    }

    pub(crate) fn mode(&self) -> BuildMode {
        self.mode
    }
}

// ---------------------------------------------------------------------------
// What the generated code calls where the macro is written
// ---------------------------------------------------------------------------

/// Asserts that `tag` constructs a host widget, as the element macros
/// assumed from its name.
#[doc(hidden)]
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a host widget",
    label = "the element macros treat this tag as a host widget by its name",
    note = "host widget names are reserved: a component cannot be called `container`, `text`, .."
)]
pub trait HostTag {}

impl<N: WidgetVariant> HostTag for Draft<N> {}

#[doc(hidden)]
pub fn host<N, R>(tag: fn() -> Draft<N, R>) -> fn() -> Draft<N, R>
where
    N: NodeDef,
    Draft<N, R>: HostTag,
{
    tag
}

/// Which host widget `tag` constructs.
#[doc(hidden)]
pub fn host_type<N: WidgetVariant>(_tag: fn() -> Draft<N>) -> WidgetType {
    N::NODE_TYPE
}

/// `value` as property `P` of the node `tag` constructs, converted now:
/// what a template's inputs hold.
#[doc(hidden)]
pub fn value<N: NodeDef, R, P: Property>(
    _tag: fn() -> Draft<N, R>,
    _property: impl FnOnce(&N::Properties) -> PropertyMarker<P>,
    value: impl IntoProp<P>,
) -> P::Value {
    value.into_prop()
}

/// [`value`] for a handler closure, which infers its argument types from the
/// property's event.
#[doc(hidden)]
pub fn handler<N: NodeDef, R, P: Property, F: Into<Flow>>(
    _tag: fn() -> Draft<N, R>,
    _property: impl FnOnce(&N::Properties) -> PropertyMarker<P>,
    handler: impl for<'a> FnMut(&<P::Value as HandlerValue>::Event, &mut EventContext<'a>) -> F
    + 'static,
) -> P::Value
where
    P::Value: HandlerValue,
{
    P::Value::from_handler(Handler::new(handler))
}

/// [`value`] for a prop forwarded with [`Share::share`](crate::prop::Share).
#[doc(hidden)]
pub fn shared<N: NodeDef, R, P, T: 'static, Via>(
    _tag: fn() -> Draft<N, R>,
    _property: impl FnOnce(&N::Properties) -> PropertyMarker<P>,
    value: impl crate::prop::IntoProp<T, Via>,
) -> P::Value
where
    P: Property<Value = crate::prop::Prop<T>>,
{
    crate::prop::IntoProp::into_prop(value)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use xui_interface::{Color, NodeId, Size};
    use xui_macros::{component, xui};

    use crate::app::App;
    use crate::render::backend::MockRenderBackend;
    use crate::state::{HookContext, State};
    use crate::test_prelude::*;
    use crate::text::TextHost;
    use crate::text::testing::ZeroTextBackend;
    use crate::widgets::WidgetType;
    use crate::{ElementDesc, EventResult};

    thread_local! {
        static FLAG: RefCell<Option<State<bool>>> = const { RefCell::new(None) };
        static COUNT: RefCell<Option<State<u32>>> = const { RefCell::new(None) };
        static ORDER: RefCell<Option<State<Vec<&'static str>>>> = const { RefCell::new(None) };
        static CHILD_RENDERS: Cell<usize> = const { Cell::new(0) };
        static UNMOUNTED: Cell<usize> = const { Cell::new(0) };
    }

    fn set<T: 'static>(slot: &'static std::thread::LocalKey<RefCell<Option<State<T>>>>, value: T) {
        let state = slot.with(|slot| slot.borrow().expect("rendered"));
        slot::Runtime::with_phase(slot::RenderPhase::Event, || state.set(value));
    }

    struct Harness {
        app: App,
        backend: MockRenderBackend,
        text: TextHost<ZeroTextBackend>,
    }

    impl Harness {
        fn new(root: crate::app::ComponentFn) -> Self {
            let mut app = App::new(root);
            app.resize(Size::new(400.0, 400.0));
            let mut harness = Self {
                app,
                backend: MockRenderBackend::default(),
                text: TextHost::new(ZeroTextBackend),
            };
            harness.render();
            harness
        }

        fn render(&mut self) {
            self.app.render(&mut self.backend, &mut self.text).expect("mock backend");
        }

        fn ui(&self) -> &crate::ui_runtime::UiRuntime {
            self.app.ui_runtime()
        }

        fn host_updates(&mut self) -> usize {
            self.app.ui_runtime_mut().stats.host_updates
        }

        /// The application's root host.
        fn top(&self) -> NodeId {
            self.ui().children(self.ui().root()).next().expect("a root element")
        }

        fn children(&self, id: NodeId) -> Vec<NodeId> {
            self.ui().children(id).collect()
        }

        fn node_type(&self, id: NodeId) -> WidgetType {
            self.ui().node(id).unwrap().node_type
        }

        fn width(&self, id: NodeId) -> f32 {
            self.ui().node(id).unwrap().layout.width()
        }
    }

    fn static_and_dynamic(cx: &mut HookContext<'_>) -> ElementDesc {
        let count = cx.use_state(|| 1u32);
        COUNT.with(|slot| *slot.borrow_mut() = Some(count));
        let width = 10.0 * *count.get() as f32;
        xui! {
            <column width={200.0} background={Color::BLACK}>
                <container width={40.0} height={10.0} />
                <container width={width} height={10.0} />
                <text>{"constant"}</text>
            </column>
        }
    }

    #[test]
    fn a_template_mounts_its_hosts_in_order() {
        let harness = Harness::new(static_and_dynamic);
        let top = harness.top();
        assert_eq!(harness.node_type(top), WidgetType::Container);
        let children = harness.children(top);
        assert_eq!(children.len(), 3);
        assert_eq!(harness.node_type(children[2]), WidgetType::Text);
        assert_eq!(harness.width(top), 200.0);
        assert_eq!(harness.width(children[0]), 40.0);
        assert_eq!(harness.width(children[1]), 10.0);
    }

    #[test]
    fn an_update_rebuilds_only_the_dynamic_hosts() {
        let mut harness = Harness::new(static_and_dynamic);
        let before = harness.children(harness.top());
        let updates = harness.host_updates();

        set(&COUNT, 3);
        harness.render();

        let after = harness.children(harness.top());
        assert_eq!(before, after, "the hosts were remounted");
        assert_eq!(harness.width(after[1]), 30.0, "the dynamic attribute did not arrive");
        assert_eq!(
            harness.host_updates() - updates,
            1,
            "only the host with a dynamic attribute is rebuilt"
        );
    }

    fn keyed_list(cx: &mut HookContext<'_>) -> ElementDesc {
        let order = cx.use_state(|| vec!["a", "b", "c"]);
        ORDER.with(|slot| *slot.borrow_mut() = Some(order));
        let rows = order
            .get()
            .iter()
            .map(|name| xui! { <text key={*name}>{*name}</text> })
            .collect::<Vec<_>>();
        xui! {
            <column>
                {rows}
            </column>
        }
    }

    fn texts(harness: &Harness, parent: NodeId) -> Vec<String> {
        harness
            .children(parent)
            .into_iter()
            .map(|id| {
                harness
                    .ui()
                    .node(id)
                    .unwrap()
                    .widget
                    .text()
                    .expect("a text")
                    .as_str()
                    .to_owned()
            })
            .collect()
    }

    #[test]
    fn a_fragment_reconciles_its_keyed_templates() {
        let mut harness = Harness::new(keyed_list);
        let top = harness.top();
        assert_eq!(texts(&harness, top), ["a", "b", "c"]);
        let before = harness.children(top);

        set(&ORDER, vec!["c", "a", "d"]);
        harness.render();

        assert_eq!(texts(&harness, top), ["c", "a", "d"]);
        let after = harness.children(top);
        assert_eq!(after[0], before[2], "a keyed template moved rather than remounted");
        assert_eq!(after[1], before[0]);
        assert!(!harness.ui().contains(before[1]), "the dropped row was removed");
    }

    struct Unmount;

    impl Drop for Unmount {
        fn drop(&mut self) {
            UNMOUNTED.with(|unmounted| unmounted.set(unmounted.get() + 1));
        }
    }

    #[component]
    fn swapping(flag: &bool) {
        CHILD_RENDERS.with(|renders| renders.set(renders.get() + 1));
        // Dropped with the component's hooks, when it unmounts.
        cx.use_ref(|| Unmount);
        if *flag {
            xui! { <text>{"swapped"}</text> }
        } else {
            xui! { <container width={5.0} /> }
        }
    }

    fn component_between_statics(cx: &mut HookContext<'_>) -> ElementDesc {
        let flag = cx.use_state(|| false);
        FLAG.with(|slot| *slot.borrow_mut() = Some(flag));
        let flag = *flag.get();
        xui! {
            <row>
                <text>{"before"}</text>
                <swapping flag={flag} />
                <text>{"after"}</text>
            </row>
        }
    }

    #[test]
    fn a_component_slot_stays_between_its_static_siblings() {
        let mut harness = Harness::new(component_between_statics);
        let top = harness.top();
        let children = harness.children(top);
        assert_eq!(children.len(), 3);
        assert_eq!(harness.node_type(children[1]), WidgetType::Container);

        // The component's root changes widget: its host is replaced, in place.
        set(&FLAG, true);
        harness.render();
        let children = harness.children(top);
        assert_eq!(children.len(), 3);
        assert_eq!(texts(&harness, top), ["before", "swapped", "after"]);
    }

    fn identity_switch(cx: &mut HookContext<'_>) -> ElementDesc {
        let flag = cx.use_state(|| false);
        FLAG.with(|slot| *slot.borrow_mut() = Some(flag));
        if *flag.get() {
            xui! { <text>{"other template"}</text> }
        } else {
            xui! { <container width={20.0} /> }
        }
    }

    #[test]
    fn a_different_template_in_the_same_place_remounts() {
        let mut harness = Harness::new(identity_switch);
        let before = harness.top();
        assert_eq!(harness.node_type(before), WidgetType::Container);

        set(&FLAG, true);
        harness.render();

        let after = harness.top();
        assert_ne!(before, after);
        assert!(!harness.ui().contains(before));
        assert_eq!(harness.node_type(after), WidgetType::Text);
    }

    fn removable(cx: &mut HookContext<'_>) -> ElementDesc {
        let flag = cx.use_state(|| true);
        FLAG.with(|slot| *slot.borrow_mut() = Some(flag));
        let shown = *flag.get();
        let child = shown.then(|| xui! { <column><swapping flag={false} /></column> });
        xui! { <container>{child}</container> }
    }

    #[test]
    fn removing_a_template_removes_its_hosts_and_unmounts_its_components() {
        let mut harness = Harness::new(removable);
        let top = harness.top();
        let column = harness.children(top)[0];
        let inner = harness.children(column)[0];
        let unmounted = UNMOUNTED.with(Cell::get);

        set(&FLAG, false);
        harness.render();

        assert!(harness.children(top).is_empty());
        assert!(!harness.ui().contains(column));
        assert!(!harness.ui().contains(inner));
        assert_eq!(UNMOUNTED.with(Cell::get), unmounted + 1, "the component did not unmount");
    }

    #[test]
    fn expanding_a_template_gives_the_tree_it_stands_for() {
        let element = xui! {
            <column gap={4.0}>
                <text>{"a"}</text>
                <container on_click={|_, _| EventResult::Consumed} />
            </column>
        };
        let ElementDesc::Host(column) = element.expanded() else {
            panic!("a host root");
        };
        let draft = column.draft::<Container>().expect("a container");
        assert_eq!(
            draft.storage.style.flex_layout.gap,
            xui_interface::StyleValue::Value(4.0.into())
        );
        assert_eq!(column.children.len(), 2);
        assert_eq!(column.children[0].node_type(), Some(WidgetType::Text));
    }

    fn adjacent_components(cx: &mut HookContext<'_>) -> ElementDesc {
        let flag = cx.use_state(|| false);
        FLAG.with(|slot| *slot.borrow_mut() = Some(flag));
        let flag = *flag.get();
        xui! {
            <row>
                <swapping flag={true} />
                <swapping flag={flag} />
                <swapping flag={true} />
            </row>
        }
    }

    #[test]
    fn a_component_slot_is_placed_before_the_next_slots_host() {
        let mut harness = Harness::new(adjacent_components);
        let top = harness.top();
        let before = harness.children(top);
        assert_eq!(harness.node_type(before[1]), WidgetType::Container);

        set(&FLAG, true);
        harness.render();

        let after = harness.children(top);
        assert_eq!(after.len(), 3);
        assert_eq!(after[0], before[0]);
        assert_eq!(after[2], before[2]);
        assert_eq!(texts(&harness, top), ["swapped", "swapped", "swapped"]);
    }

    #[component]
    fn menu_owner(open: &bool) {
        let menu: Option<ElementDesc> = open.then(|| {
            crate::element::portal(vec![xui! { <text key="menu">{"menu"}</text> }]).into()
        });
        xui! {
            <container width={10.0}>
                {menu}
            </container>
        }
    }

    fn with_portal(cx: &mut HookContext<'_>) -> ElementDesc {
        let flag = cx.use_state(|| true);
        FLAG.with(|slot| *slot.borrow_mut() = Some(flag));
        let open = *flag.get();
        xui! {
            <column>
                <menu_owner open={open} />
            </column>
        }
    }

    #[test]
    fn a_template_in_a_portal_mounts_under_the_overlayer() {
        let mut harness = Harness::new(with_portal);
        let overlayer = harness.ui().root_overlayer();
        let layers = harness.children(overlayer);
        assert_eq!(layers.len(), 1);
        assert_eq!(texts(&harness, overlayer), ["menu"]);

        set(&FLAG, false);
        harness.render();
        assert!(harness.children(overlayer).is_empty());
        assert!(!harness.ui().contains(layers[0]));
    }
}
