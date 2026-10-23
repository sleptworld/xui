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
//! all together, and on a later render with the same template writes only
//! the dynamic attributes whose values changed. The static hosts are not
//! visited, and a render that changed no value leaves the template out of
//! the commit altogether. To tell, a mounted template keeps the inputs of the
//! last render that wrote into it, and compares each value with the one it
//! wrote: a value of a type without `PartialEq` -- a handler -- counts as
//! changed every time.
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
//! [`TemplateDesc::expand`] produces an inspection-only [`Expanded`] tree.
//! Rendering always uses the template; expanded hosts are not render inputs.

use std::any::{Any, TypeId};
use std::cell::RefCell;

use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use std::marker::PhantomData;

use xui_interface::{NodeId, WidgetUpdateFlags};

use crate::capability::CommitCx;
use crate::caps::{Columns, Registered};
use crate::element::{
    ComponentDesc, ComponentElement, ComponentProps, ElementDesc, Expanded, ExpandedHost,
    FragmentDesc,
};
use crate::event_system::{EventContext, Flow, Handler, HandlerValue};
use crate::fiber::{ComponentRender, ErasedProps, Key};
use crate::node::{Draft, NodeDef, OwnProps, WholeNode, WidgetInstance, WidgetT};
use crate::property::{CapScope, IntoProp, Property, PropertyMarker, Scope};
use crate::ui_runtime::UiRuntime;
use crate::widgets::{ErasedDraft, WidgetType, WidgetVariant};

/// The static structure of one `xui!` / `view!` call site.
#[derive(Debug)]
pub struct TemplateMeta {
    /// Where it was written, for diagnostics.
    pub location: &'static str,
    /// Every node, in preorder: a parent comes before its children, and
    /// siblings appear in the order written.
    pub nodes: &'static [TemplateNode],
    /// Every attribute of a host that is not a constant: where each of the
    /// template's dynamic values goes, in the order the template writes
    /// them. An update writes these and nothing else of its hosts, and
    /// names the ones it changed by their index here.
    pub values: &'static [ValueSlot],
}

/// A dynamic attribute of one of a template's hosts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueSlot {
    /// The host's index among the template's nodes.
    pub node: u16,
    /// The attribute, as written.
    pub attribute: &'static str,
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
    /// A host node the template owns. Whether an update builds it again --
    /// whether it has an attribute that is not a constant -- is decided in
    /// the build function the macro generates, not looked up here.
    Host { tag: &'static str },
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

trait ErasedTemplate: Any {
    /// `old` is the instance the template committed last, of the same type.
    fn build(&mut self, old: Option<&dyn ErasedTemplate>, out: &mut TemplateBuild);
    fn commit(&mut self, hosts: &mut TemplateHosts<'_>);
    fn clone_box(&self) -> Box<dyn ErasedTemplate>;
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
    fn as_any(&self) -> &dyn Any;
    /// Drops the inputs, so that an instance waiting to be reused holds
    /// nothing of the application's.
    fn clear(&mut self);
}

/// How many committed instances of one type are kept for reuse.
const RECYCLED_PER_TYPE: usize = 4096;

thread_local! {
    /// Instances done with, by type, their inputs dropped. The next render
    /// of the same template -- the next frame's, nearly always -- takes one
    /// instead of allocating.
    static RECYCLED: RefCell<FxHashMap<TypeId, Vec<Box<dyn ErasedTemplate>>>> =
        RefCell::new(FxHashMap::default());
}

/// A template's generated build function: this render's inputs, the inputs
/// of the render it committed last, if it kept them, and what to fill in.
type BuildFn<I> = fn(&mut I, Option<&I>, &mut TemplateBuild);

/// One render's inputs, and the two functions the macro generated for its
/// call site. Every input is an `Option`. When the template mounts, `build`
/// takes them all. On an update it takes the slots', and copies a host's
/// dynamic values only where that host is rebuilt whole; `commit` copies
/// those it writes into the mounted hosts. What an update leaves is what the
/// template keeps, to compare the next render's values with.
struct Instance<I> {
    /// `None` only while the instance waits to be reused.
    inputs: Option<I>,
    build: BuildFn<I>,
    commit: fn(&mut I, &mut TemplateHosts<'_>),
}

impl<I: Clone + 'static> Instance<I> {
    /// A boxed instance: a recycled one of the same type when there is one.
    fn boxed(
        inputs: I,
        build: BuildFn<I>,
        commit: fn(&mut I, &mut TemplateHosts<'_>),
    ) -> Box<dyn ErasedTemplate> {
        let inputs = Some(inputs);
        let recycled = RECYCLED.with(|recycled| {
            recycled
                .borrow_mut()
                .get_mut(&TypeId::of::<Self>())
                .and_then(Vec::pop)
        });
        match recycled.map(|instance| instance.into_any().downcast::<Self>()) {
            Some(Ok(mut instance)) => {
                *instance = Instance {
                    inputs,
                    build,
                    commit,
                };
                instance
            }
            _ => Box::new(Instance {
                inputs,
                build,
                commit,
            }),
        }
    }
}

impl<I: Clone + 'static> ErasedTemplate for Instance<I> {
    fn build(&mut self, old: Option<&dyn ErasedTemplate>, out: &mut TemplateBuild) {
        let old = old
            .and_then(|old| old.as_any().downcast_ref::<Self>())
            .and_then(|old| old.inputs.as_ref());
        let inputs = self
            .inputs
            .as_mut()
            .expect("a template renders its own inputs");
        (self.build)(inputs, old, out);
    }

    fn commit(&mut self, hosts: &mut TemplateHosts<'_>) {
        let inputs = self
            .inputs
            .as_mut()
            .expect("a template commits its own inputs");
        (self.commit)(inputs, hosts);
    }

    fn clone_box(&self) -> Box<dyn ErasedTemplate> {
        Box::new(Instance {
            inputs: self.inputs.clone(),
            build: self.build,
            commit: self.commit,
        })
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clear(&mut self) {
        self.inputs = None;
    }
}

/// Puts `instance` aside for the next render of its template, its inputs
/// dropped.
fn recycle(mut instance: Box<dyn ErasedTemplate>) {
    instance.clear();
    let ty = Any::type_id(&*instance);
    RECYCLED.with(|recycled| {
        let mut recycled = recycled.borrow_mut();
        let instances = recycled.entry(ty).or_default();
        if instances.len() < RECYCLED_PER_TYPE {
            instances.push(instance);
        }
    });
}

/// The inputs of the render a mounted template last wrote into its hosts:
/// what the next render's values are compared with.
pub(crate) struct CommittedTemplate(Box<dyn ErasedTemplate>);

impl CommittedTemplate {
    /// Puts the instance aside for reuse, its inputs dropped.
    pub(crate) fn recycle(self) {
        recycle(self.0);
    }
}

impl TemplateDesc {
    /// A render of the template at `meta` with `inputs`, which `build`
    /// turns into the template's drafts and slots, and `commit` writes into
    /// its mounted hosts. Generated by the element macros.
    #[doc(hidden)]
    pub fn new<I: Clone + 'static>(
        meta: &'static TemplateMeta,
        key: Option<Key>,
        root_type: WidgetType,
        inputs: I,
        build: fn(&mut I, Option<&I>, &mut TemplateBuild),
        commit: fn(&mut I, &mut TemplateHosts<'_>),
    ) -> Self {
        Self {
            meta,
            identity: TemplateIdentity {
                meta,
                inputs: TypeId::of::<I>(),
            },
            key,
            root_type,
            instance: Instance::boxed(inputs, build, commit),
        }
    }

    /// Hands the instance back for the next render of this template, once
    /// nothing needs its inputs.
    pub(crate) fn recycle(self) {
        recycle(self.instance);
    }

    /// The inputs this render left, for the template to keep once it is
    /// committed.
    pub(crate) fn into_committed(self) -> CommittedTemplate {
        CommittedTemplate(self.instance)
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
    /// expands; and every slot. On an update it compares each dynamic value
    /// with `committed`'s, notes those that changed, and builds the drafts
    /// only of hosts rebuilt whole that have one; the other hosts' changed
    /// values stay, for [`commit`](Self::commit).
    pub(crate) fn build(
        &mut self,
        mode: BuildMode,
        committed: Option<&CommittedTemplate>,
    ) -> TemplateBuild {
        let mut out = TemplateBuild {
            mode,
            hosts: Vec::new(),
            slots: Vec::new(),
            changed: ChangedValues::default(),
        };
        self.instance
            .build(committed.map(|committed| &*committed.0), &mut out);
        out
    }

    /// Writes an update's dynamic values into the mounted hosts, after
    /// [`build`](Self::build) ran in [`BuildMode::Update`].
    pub(crate) fn commit(&mut self, hosts: &mut TemplateHosts<'_>) {
        self.instance.commit(hosts);
    }

    /// The hosts this template holds, each with its draft and what is
    /// under it: each slot in its place, expanded too.
    pub fn expand(mut self) -> Expanded {
        let meta = self.meta;
        let mut built = self.build(BuildMode::Mount, None);
        let mut drafts = std::mem::take(&mut built.hosts).into_iter();
        let mut slots = std::mem::take(&mut built.slots).into_iter();
        // Each node's expansion: a host's, or a slot's (a fragment's spliced).
        let mut expanded: Vec<Vec<Expanded>> = (0..meta.nodes.len()).map(|_| Vec::new()).collect();
        // Slots arrive in node order.
        for (index, node) in meta.nodes.iter().enumerate() {
            if !matches!(node.kind, TemplateNodeKind::Host { .. }) {
                let element = match slots.next() {
                    Some(TemplateSlot::Element(element)) => element,
                    Some(TemplateSlot::Component { render, props, key }) => {
                        ElementDesc::Component(ComponentDesc { key, render, props })
                    }
                    Some(TemplateSlot::Keep) | None => {
                        panic!("template {} lost a slot", meta.location)
                    }
                };
                element.expand_into(&mut expanded[index]);
            }
        }
        // Children come after their parent, so assembling back to front
        // finds every child finished.
        for index in (0..meta.nodes.len()).rev() {
            if !matches!(meta.nodes[index].kind, TemplateNodeKind::Host { .. }) {
                continue;
            }
            let children = (index + 1..meta.nodes.len())
                .filter(|&child| meta.nodes[child].parent == Some(index as u16))
                .flat_map(|child| std::mem::take(&mut expanded[child]))
                .collect();
            // Drafts arrive in node order, so back to front is in reverse.
            let draft = drafts
                .next_back()
                .filter(|(at, _)| usize::from(*at) == index)
                .map(|(_, draft)| draft)
                .unwrap_or_else(|| panic!("template {} built no draft for a host", meta.location));
            expanded[index] = vec![Expanded::Host(ExpandedHost { draft, children })];
        }
        expanded[0].pop().expect("a template's root is a host")
    }

    /// A draft written by hand, with `children`, as a template of one host:
    /// rebuilt whole on every update, its children a fragment.
    pub(crate) fn draft<N>(draft: Draft<N>, children: Vec<ElementDesc>) -> Self
    where
        N: WidgetVariant,
        N::Props: Clone,
        N::Behavior: Clone,
    {
        static DRAFT: TemplateMeta = TemplateMeta {
            location: "a draft",
            nodes: &[
                TemplateNode {
                    kind: TemplateNodeKind::Host { tag: "draft" },
                    parent: None,
                },
                TemplateNode {
                    kind: TemplateNodeKind::Fragment,
                    parent: Some(0),
                },
            ],
            values: &[],
        };
        type Inputs<N> = (Option<Draft<N>>, Option<Vec<ElementDesc>>);
        fn build<N>(
            (draft, children): &mut Inputs<N>,
            _: Option<&Inputs<N>>,
            out: &mut TemplateBuild,
        ) where
            N: WidgetVariant,
            N::Props: Clone,
            N::Behavior: Clone,
        {
            out.host(0, take(draft));
            out.fragment(take(children));
        }
        fn commit<N: NodeDef>(_: &mut Inputs<N>, _: &mut TemplateHosts<'_>) {}
        let key = draft.key.clone();
        Self::new(
            &DRAFT,
            key,
            N::NODE_TYPE,
            (Some(draft), Some(children)),
            build::<N>,
            commit::<N>,
        )
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

/// What a pass's reconcile of a template leaves for the commit.
pub(crate) struct TemplatePending {
    pub(crate) mode: BuildMode,
    /// The render, kept for its inputs: on an update, the commit writes the
    /// dynamic values left in them.
    pub(crate) desc: TemplateDesc,
    /// The hosts built whole, by node index, in node order.
    pub(crate) hosts: Vec<(u16, Box<dyn ErasedDraft>)>,
    /// On an update, the dynamic values that changed.
    pub(crate) changed: ChangedValues,
}

/// A slot's content, one render's worth.
pub(crate) enum TemplateSlot {
    /// A component, its render function known from its props' type.
    Component {
        render: ComponentRender,
        props: Option<ErasedProps>,
        key: Option<Key>,
    },
    /// Anything else written where a template holds no host: a child list
    /// as a fragment, a portal, an element built by hand.
    Element(ElementDesc),
    /// A component whose attributes are all constants, on an update: it is
    /// what it was, so its fiber is kept as it is.
    Keep,
}

impl TemplateSlot {
    /// The element in the slot; `None` for [`Keep`](Self::Keep).
    pub(crate) fn into_element(self) -> Option<ElementDesc> {
        match self {
            Self::Component { render, props, key } => {
                Some(ElementDesc::Component(ComponentDesc { key, render, props }))
            }
            Self::Element(element) => Some(element),
            Self::Keep => None,
        }
    }
}

/// Which of a template's dynamic values an update changed, by their index in
/// [`TemplateMeta::values`].
#[derive(Debug, Default)]
pub(crate) struct ChangedValues(SmallVec<[u64; 1]>);

impl ChangedValues {
    fn insert(&mut self, value: u16) {
        let (word, bit) = (usize::from(value / 64), value % 64);
        if self.0.len() <= word {
            self.0.resize(word + 1, 0);
        }
        self.0[word] |= 1 << bit;
    }

    pub(crate) fn contains(&self, value: u16) -> bool {
        self.0
            .get(usize::from(value / 64))
            .is_some_and(|word| word & (1 << (value % 64)) != 0)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.iter().all(|word| *word == 0)
    }
}

/// What a template's build function fills in. Public only because generated
/// code names it.
#[doc(hidden)]
pub struct TemplateBuild {
    mode: BuildMode,
    /// The hosts built whole, by node index, in node order: every host on
    /// mount, on an update only those a patch cannot follow.
    pub(crate) hosts: Vec<(u16, Box<dyn ErasedDraft>)>,
    pub(crate) slots: Vec<TemplateSlot>,
    /// On an update, the dynamic values that changed.
    pub(crate) changed: ChangedValues,
}

impl TemplateBuild {
    /// Host `index`'s draft.
    pub fn host<N>(&mut self, index: u16, draft: Draft<N>)
    where
        N: WidgetVariant,
        N::Props: Clone,
        N::Behavior: Clone,
    {
        self.hosts.push((index, Box::new(draft)));
    }

    /// The next slot: what a tag the template holds no host for built.
    pub fn slot<S: SlotInput>(&mut self, input: S) {
        input.fill(self);
    }

    /// The next slot: a child list.
    pub fn fragment(&mut self, children: Vec<ElementDesc>) {
        self.slots
            .push(TemplateSlot::Element(FragmentDesc::new(children).into()));
    }

    /// Whether this build mounts (or expands) the template.
    pub fn mounting(&self) -> bool {
        self.mode == BuildMode::Mount
    }

    /// Notes whether dynamic value `value`, its index in
    /// [`TemplateMeta::values`], changed; returns `changed`.
    pub fn note(&mut self, value: u16, changed: bool) -> bool {
        if changed {
            self.changed.insert(value);
        }
        changed
    }

    /// Whether an update built nothing to write: no host rebuilt and no
    /// value changed. Its slots are reconciled all the same.
    pub(crate) fn writes_nothing(&self) -> bool {
        self.hosts.is_empty() && self.changed.is_empty()
    }

    /// The next slot: a constant component, unchanged since the template
    /// mounted. Only on an update.
    pub fn keep(&mut self) {
        debug_assert!(!self.mounting(), "a template mounts every component");
        self.slots.push(TemplateSlot::Keep);
    }
}

// ---------------------------------------------------------------------------
// Slots
// ---------------------------------------------------------------------------

/// What a tag written where a template holds no host builds, as the
/// template's slot takes it. Public only because generated code names it.
#[doc(hidden)]
pub trait SlotContent: Into<ElementDesc> {
    /// What the template keeps of it as an input: an input is cloneable,
    /// and component props need not be.
    type Input: SlotInput;

    fn into_input(self) -> Self::Input;
}

/// A slot's input, which fills the slot when the template builds.
#[doc(hidden)]
pub trait SlotInput: Clone + 'static {
    fn fill(self, out: &mut TemplateBuild);
}

impl SlotContent for ElementDesc {
    type Input = ElementDesc;

    fn into_input(self) -> ElementDesc {
        self
    }
}

impl SlotInput for ElementDesc {
    fn fill(self, out: &mut TemplateBuild) {
        out.slots.push(TemplateSlot::Element(self));
    }
}

/// A component slot's input: the props, already handed over to the fiber's
/// allocation, and their type, which names the component. Being part of the
/// template's input type, that type is part of the template's identity, so a
/// template matched to the one it committed holds the same component in the
/// same slot.
#[doc(hidden)]
pub struct ComponentSlot<P> {
    props: Option<ErasedProps>,
    key: Option<Key>,
    component: PhantomData<fn() -> P>,
}

impl<P> Clone for ComponentSlot<P> {
    fn clone(&self) -> Self {
        Self {
            props: self.props.clone(),
            key: self.key,
            component: PhantomData,
        }
    }
}

impl<P: ComponentProps> SlotContent for ComponentElement<P> {
    type Input = ComponentSlot<P>;

    fn into_input(self) -> ComponentSlot<P> {
        ComponentSlot {
            props: self.props.erase(),
            key: self.key,
            component: PhantomData,
        }
    }
}

impl<P: ComponentProps> SlotInput for ComponentSlot<P> {
    fn fill(self, out: &mut TemplateBuild) {
        out.slots.push(TemplateSlot::Component {
            render: P::RENDER,
            props: self.props,
            key: self.key,
        });
    }
}

/// Takes a template input out of the inputs: a slot's, or a value when the
/// template mounts, used once.
#[doc(hidden)]
#[inline(always)]
pub fn take<T>(input: &mut Option<T>) -> T {
    input.take().expect("a template input is used once")
}

/// A dynamic value for a host's draft: taken when the template mounts; on
/// an update a copy, the input kept to compare the next render's with.
#[doc(hidden)]
#[inline(always)]
pub fn arg<T: Clone>(mounting: bool, input: &mut Option<T>) -> T {
    if mounting { take(input) } else { cloned(input) }
}

/// A copy of a dynamic value, for the commit to write into a mounted host.
#[doc(hidden)]
#[inline(always)]
pub fn cloned<T: Clone>(input: &Option<T>) -> T {
    input.clone().expect("an update keeps its values")
}

/// Whether a dynamic value differs from what the template last wrote:
/// `Compare(new, old).changed()`, with [`CompareEq`] and [`CompareAny`] in
/// scope, compares a value whose type has `PartialEq`, and calls any other
/// changed. A value never written -- a template mounts with its values
/// taken -- is changed too.
#[doc(hidden)]
pub struct Compare<'a, T>(pub &'a Option<T>, pub Option<&'a T>);

#[doc(hidden)]
pub trait CompareEq {
    fn changed(&self) -> bool;
}

impl<T: PartialEq> CompareEq for Compare<'_, T> {
    #[inline(always)]
    fn changed(&self) -> bool {
        match (self.0, self.1) {
            (Some(new), Some(old)) => new != old,
            _ => true,
        }
    }
}

/// [`CompareEq`]'s fallback, found by auto-ref for a type without
/// `PartialEq`.
#[doc(hidden)]
pub trait CompareAny {
    fn changed(&self) -> bool;
}

impl<T> CompareAny for &Compare<'_, T> {
    #[inline(always)]
    fn changed(&self) -> bool {
        true
    }
}

/// `patch()` when `changed`, nothing otherwise.
#[doc(hidden)]
#[inline(always)]
pub fn patch_if(changed: bool, patch: impl FnOnce() -> WidgetUpdateFlags) -> WidgetUpdateFlags {
    if changed {
        patch()
    } else {
        WidgetUpdateFlags::empty()
    }
}

/// A template input, still in the inputs.
#[doc(hidden)]
#[inline(always)]
pub fn input<T>(input: &Option<T>) -> &T {
    input.as_ref().expect("a template input is used once")
}

/// The type of a constant slot's input, as a template input of its own:
/// `build` is never called. A constant slot is built only when the template
/// mounts, so nothing else puts its type among the inputs, and without it a
/// generic function's template, instantiated for another component, would
/// keep the first one in the slot.
#[doc(hidden)]
#[inline(always)]
pub fn slot_kind<S: SlotInput>(_build: impl FnOnce() -> S) -> PhantomData<fn() -> S> {
    PhantomData
}

// ---------------------------------------------------------------------------
// Patching a mounted host
// ---------------------------------------------------------------------------

/// A mounted template's hosts, as its generated `commit` writes an update's
/// dynamic values into them. Public only because generated code names it.
#[doc(hidden)]
pub struct TemplateHosts<'a> {
    arena: &'a mut UiRuntime,
    /// The mounted hosts, by node index.
    nodes: &'a [Option<NodeId>],
    changed: &'a ChangedValues,
}

impl<'a> TemplateHosts<'a> {
    pub(crate) fn new(
        arena: &'a mut UiRuntime,
        nodes: &'a [Option<NodeId>],
        changed: &'a ChangedValues,
    ) -> Self {
        Self {
            arena,
            nodes,
            changed,
        }
    }

    /// Whether the update changed dynamic value `value`, its index in
    /// [`TemplateMeta::values`].
    pub fn changed(&self, value: u16) -> bool {
        self.changed.contains(value)
    }

    /// Writes host `index`'s dynamic values: `patch` sets them on the host,
    /// of `tag`'s type, through a [`HostPatcher`] and returns what that
    /// dirtied.
    pub fn patch<N, F>(&mut self, index: u16, _tag: fn() -> Draft<N>, patch: F)
    where
        N: WidgetVariant,
        F: FnOnce(&mut HostPatcher<'_, '_, N>) -> WidgetUpdateFlags,
    {
        let id = self.nodes[usize::from(index)].expect("a mounted template keeps its hosts");
        self.arena.patch_node(id, |widgets, _, columns, cx| {
            let instance = N::unwrap_mut(widgets).expect("a template patches its own host");
            patch(&mut HostPatcher {
                instance,
                id,
                columns,
                cx,
            })
        });
    }
}

/// A mounted host of type `N`, as a template's update writes its dynamic
/// attributes into it: its own props and state, and its capabilities' rows.
/// Public only because generated code names it.
#[doc(hidden)]
pub struct HostPatcher<'a, 'cx, N: WidgetT> {
    instance: &'a mut WidgetInstance<N>,
    id: NodeId,
    columns: &'a mut Columns,
    cx: &'a mut CommitCx<'cx>,
}

impl<N: WidgetT> HostPatcher<'_, '_, N> {
    /// The host's property table, as the element macros look attributes up.
    #[inline(always)]
    pub fn __properties(&self) -> N::Properties {
        N::Properties::default()
    }

    /// Writes `value` as property `P`; returns what that dirtied.
    #[inline]
    pub fn __patch<P: Property>(&mut self, _: PropertyMarker<P>, value: P::Value) -> WidgetUpdateFlags
    where
        P::Scope: PatchScope<N>,
    {
        <P::Scope as PatchScope<N>>::patch::<P>(self, value)
    }
}

/// How a property of scope `Self` is written into a mounted host of type
/// `N`, if it can be. Every scope a host property can have implements it, so
/// that the element macros can ask any attribute whether it patches.
#[doc(hidden)]
pub trait PatchScope<N: WidgetT>: Scope + Sized {
    /// Whether a property of this scope can be written into a mounted host
    /// alone. When one of a host's dynamic attributes cannot, the host is
    /// rebuilt whole.
    const PATCHABLE: bool;

    fn patch<P: Property<Scope = Self>>(
        target: &mut HostPatcher<'_, '_, N>,
        value: P::Value,
    ) -> WidgetUpdateFlags;
}

/// A capability field: the row with the field replaced, written back the way
/// a scatter writes it, which compares, commits and reports.
impl<N: WidgetT, C: Registered> PatchScope<N> for CapScope<C>
where
    C::Storage: Clone,
{
    const PATCHABLE: bool = true;

    fn patch<P: Property<Scope = Self>>(
        target: &mut HostPatcher<'_, '_, N>,
        value: P::Value,
    ) -> WidgetUpdateFlags {
        let mut row = C::get(target.columns, target.id)
            .cloned()
            .unwrap_or_default();
        P::set(&mut row, value);
        C::write(target.columns, target.id, row, target.cx)
    }
}

/// One of the host's own props: replaced in place, then compared and handed
/// to the widget's state as a whole update would.
impl<N: WidgetT> PatchScope<N> for OwnProps<N>
where
    N::Props: Clone,
{
    const PATCHABLE: bool = true;

    fn patch<P: Property<Scope = Self>>(
        target: &mut HostPatcher<'_, '_, N>,
        value: P::Value,
    ) -> WidgetUpdateFlags {
        let instance = &mut *target.instance;
        let old = instance.props.clone();
        P::set(&mut instance.props, value);
        N::props_dirty(&old, &instance.props) | N::update(&old, &instance.props, &mut instance.state)
    }
}

/// `style`, which layers declarations and rules over the whole node: only a
/// rebuild knows what it replaced.
impl<N: WidgetT> PatchScope<N> for WholeNode<N> {
    const PATCHABLE: bool = false;

    fn patch<P: Property<Scope = Self>>(
        _: &mut HostPatcher<'_, '_, N>,
        _: P::Value,
    ) -> WidgetUpdateFlags {
        unreachable!("a host with a `style` attribute is rebuilt, never patched")
    }
}

// ---------------------------------------------------------------------------
// What the generated code calls where the macro is written
// ---------------------------------------------------------------------------

/// Whether attribute `property` of a host of `tag`'s type can be patched.
#[doc(hidden)]
#[inline(always)]
pub fn patchable<N: WidgetT, P: Property>(
    _tag: fn() -> Draft<N>,
    _property: impl FnOnce(&N::Properties) -> PropertyMarker<P>,
) -> bool
where
    P::Scope: PatchScope<N>,
{
    <P::Scope as PatchScope<N>>::PATCHABLE
}

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

    use xui_interface::{Color, EdgeInsets, NodeId, Size};
    use xui_macros::{component, style, view, xui};

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
        static TICKS: Cell<u32> = const { Cell::new(0) };
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
            self.app
                .render(&mut self.backend, &mut self.text)
                .expect("mock backend");
        }

        fn ui(&self) -> &crate::ui_runtime::UiRuntime {
            self.app.ui_runtime()
        }

        fn host_updates(&mut self) -> usize {
            self.app.ui_runtime_mut().stats.host_updates
        }

        /// The application's root host.
        fn top(&self) -> NodeId {
            self.ui()
                .children(self.ui().root())
                .next()
                .expect("a root element")
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
        assert_eq!(
            harness.width(after[1]),
            30.0,
            "the dynamic attribute did not arrive"
        );
        assert_eq!(
            harness.host_updates() - updates,
            1,
            "only the host with a dynamic attribute is rebuilt"
        );
    }

    fn unchanged_values(cx: &mut HookContext<'_>) -> ElementDesc {
        let count = cx.use_state(|| 1u32);
        COUNT.with(|slot| *slot.borrow_mut() = Some(count));
        // Renders again without changing anything the template shows.
        let flag = cx.use_state(|| false);
        FLAG.with(|slot| *slot.borrow_mut() = Some(flag));
        let _ = *flag.get();
        let count = *count.get();
        let card = style!(container {
            background: Color::BLACK
        });
        xui! {
            <column width={200.0}>
                <container width={10.0 * count as f32} height={10.0} />
                <text>{format!("count {count}")}</text>
                <container style={card} />
            </column>
        }
    }

    #[test]
    fn an_update_writes_only_the_values_that_changed() {
        let mut harness = Harness::new(unchanged_values);
        let toggle = |harness: &mut Harness| {
            let on = FLAG.with(|slot| *slot.borrow().expect("rendered").get());
            set(&FLAG, !on);
            harness.render();
        };

        // The first update has nothing to compare with: every dynamic host
        // is written.
        let updates = harness.host_updates();
        toggle(&mut harness);
        assert_eq!(harness.host_updates() - updates, 3);

        let updates = harness.host_updates();
        toggle(&mut harness);
        assert_eq!(
            harness.host_updates() - updates,
            0,
            "a render with the same values wrote into the hosts"
        );

        let updates = harness.host_updates();
        set(&COUNT, 2);
        harness.render();
        assert_eq!(
            harness.host_updates() - updates,
            2,
            "the width and the text changed, the style did not"
        );
        let children = harness.children(harness.top());
        assert_eq!(harness.width(children[0]), 20.0);
        let text = harness.ui().node(children[1]).unwrap().widget.text();
        assert_eq!(
            text.map(|text| text.as_str().to_owned()).as_deref(),
            Some("count 2")
        );

        let updates = harness.host_updates();
        toggle(&mut harness);
        assert_eq!(harness.host_updates() - updates, 0);
    }

    #[test]
    fn a_value_is_compared_when_its_type_can_be() {
        use super::{Compare, CompareAny as _, CompareEq as _};

        assert!(!(&Compare(&Some(1), Some(&1))).changed());
        assert!((&Compare(&Some(1), Some(&2))).changed());
        assert!(
            (&Compare(&Some(1), None)).changed(),
            "a value never written"
        );

        struct Opaque;
        assert!(
            (&Compare(&Some(Opaque), Some(&Opaque))).changed(),
            "a type without `PartialEq` counts as changed"
        );
    }

    thread_local! {
        static CLICKS: Cell<u32> = const { Cell::new(0) };
    }

    fn click_counter(cx: &mut HookContext<'_>) -> ElementDesc {
        let count = cx.use_state(|| 0u32);
        let seen = *count.get();
        CLICKS.with(|clicks| clicks.set(seen));
        xui! {
            <container
                width={400.0}
                height={300.0}
                on_click={move |_, _| {
                    count.set(seen + 1);
                    EventResult::Consumed
                }}
            />
        }
    }

    #[test]
    fn a_handler_is_written_again_on_every_update() {
        use std::time::{Duration, Instant};
        use xui_interface::Point;
        use xui_interface::events::{
            Modifiers, PointerButton, PointerButtons, PointerKind, RawEvent, RawPointerButton,
            XuiPointerId,
        };

        use crate::runtime::{GuiRuntime, RuntimeEvent};

        // A handler is never equal to the last one, and each captures the
        // count its render saw: clicking counts on only if every update
        // writes the new one.
        let mut runtime = GuiRuntime::new(
            App::new(click_counter),
            MockRenderBackend::default(),
            ZeroTextBackend,
        );
        runtime.handle_event(RuntimeEvent::Resize(Size::new(400.0, 300.0)));
        let start = Instant::now();
        runtime.frame_at(start).expect("mock backend");
        for clicks in 1..=3u32 {
            let at = start + Duration::from_millis(u64::from(clicks) * 100);
            for down in [true, false] {
                let raw = RawPointerButton {
                    position: Point::new(10.0, 10.0),
                    pointer_id: XuiPointerId::new(0),
                    device_id: None,
                    kind: PointerKind::Mouse,
                    button: PointerButton::Primary,
                    buttons: PointerButtons::default(),
                    modifiers: Modifiers::default(),
                    timestamp: at,
                };
                runtime.handle_event(RuntimeEvent::Input(if down {
                    RawEvent::PointerDown(raw)
                } else {
                    RawEvent::PointerUp(raw)
                }));
            }
            runtime.frame_at(at).expect("mock backend");
            assert_eq!(CLICKS.with(Cell::get), clicks);
        }
    }

    fn conditional_middle(cx: &mut HookContext<'_>) -> ElementDesc {
        let flag = cx.use_state(|| false);
        FLAG.with(|slot| *slot.borrow_mut() = Some(flag));
        let on = *flag.get();
        view! {
            column {
                text("header")
                if on {
                    text("on")
                }
                text("footer")
            }
        }
    }

    #[test]
    fn the_elements_around_a_condition_stay_in_the_template() {
        let mut harness = Harness::new(conditional_middle);
        let before = harness.children(harness.top());
        assert_eq!(texts(&harness, harness.top()), ["header", "footer"]);

        set(&FLAG, true);
        harness.render();
        let after = harness.children(harness.top());
        assert_eq!(texts(&harness, harness.top()), ["header", "on", "footer"]);
        assert_eq!(
            (after[0], after[2]),
            (before[0], before[1]),
            "the static siblings were mounted again"
        );

        set(&FLAG, false);
        harness.render();
        assert_eq!(harness.children(harness.top()), before);
    }

    fn scoped_let(_: &mut HookContext<'_>) -> ElementDesc {
        let label = "outer";
        view! {
            column {
                row {
                    let label = "inner";
                    text(label)
                }
                text(label)
            }
        }
    }

    #[test]
    fn a_let_among_children_is_in_scope_for_its_siblings_only() {
        let harness = Harness::new(scoped_let);
        let top = harness.top();
        let children = harness.children(top);
        assert_eq!(texts(&harness, children[0]), ["inner"]);
        let outer = harness.ui().node(children[1]).unwrap().widget.text();
        assert_eq!(outer.map(|text| text.as_str().to_owned()).as_deref(), Some("outer"));
    }

    #[component]
    fn typed_slot<T: 'static>(label: &String) {
        cx.use_ref(|| Unmount);
        xui! { <text>{format!("{label} {}", std::any::type_name::<T>())}</text> }
    }

    /// The same call site, instantiated for another type: the template is the
    /// same, its slot's component is not.
    fn typed_holder<T: 'static>() -> ElementDesc {
        xui! {
            <column>
                <typed_slot::<T> label={String::from("slot")} />
            </column>
        }
    }

    fn typed_switch(cx: &mut HookContext<'_>) -> ElementDesc {
        let flag = cx.use_state(|| false);
        FLAG.with(|slot| *slot.borrow_mut() = Some(flag));
        if *flag.get() {
            typed_holder::<u16>()
        } else {
            typed_holder::<u8>()
        }
    }

    #[test]
    fn a_slot_of_another_instantiation_remounts_its_component() {
        let mut harness = Harness::new(typed_switch);
        let unmounted = UNMOUNTED.with(Cell::get);

        set(&FLAG, true);
        harness.render();

        assert_eq!(
            UNMOUNTED.with(Cell::get) - unmounted,
            1,
            "`typed_slot::<u16>` took over the fiber of `typed_slot::<u8>`"
        );
        assert_eq!(texts(&harness, harness.top()), ["slot u16"]);
    }

    #[component(memo)]
    fn memo_label(label: &String, size: &f32) {
        CHILD_RENDERS.with(|renders| renders.set(renders.get() + 1));
        xui! { <text font_size={*size}>{label.clone()}</text> }
    }

    fn memo_parent(cx: &mut HookContext<'_>) -> ElementDesc {
        let count = cx.use_state(|| 1u32);
        COUNT.with(|slot| *slot.borrow_mut() = Some(count));
        let label = if *count.get() < 3 { "low" } else { "high" };
        xui! {
            <column>
                <memo_label label={label.to_string()} size={12.0 + 0.0 * *count.get() as f32} />
            </column>
        }
    }

    #[test]
    fn a_memo_component_renders_again_only_when_its_props_change() {
        let mut harness = Harness::new(memo_parent);
        let renders = CHILD_RENDERS.with(Cell::get);

        set(&COUNT, 2);
        harness.render();
        assert_eq!(
            CHILD_RENDERS.with(Cell::get),
            renders,
            "equal props, built anew by the parent, rendered the memo component again"
        );

        set(&COUNT, 3);
        harness.render();
        assert_eq!(CHILD_RENDERS.with(Cell::get), renders + 1);
        assert_eq!(texts(&harness, harness.top()), ["high"]);
    }

    /// A clock-like source: every call answers something new, as
    /// `Instant::now()` would.
    struct Ticks;

    impl Ticks {
        fn next() -> f32 {
            TICKS.with(|ticks| {
                ticks.set(ticks.get() + 1);
                ticks.get() as f32
            })
        }
    }

    fn impure_call(cx: &mut HookContext<'_>) -> ElementDesc {
        let count = cx.use_state(|| 1u32);
        COUNT.with(|slot| *slot.borrow_mut() = Some(count));
        let _ = count.get();
        xui! {
            <column>
                <container width={Ticks::next()} height={10.0} />
                <container width={EdgeInsets::all(3.0).left} height={10.0} />
            </column>
        }
    }

    #[test]
    fn a_call_that_may_be_impure_is_evaluated_every_render() {
        let mut harness = Harness::new(impure_call);
        let children = harness.children(harness.top());
        let first = harness.width(children[0]);

        set(&COUNT, 2);
        harness.render();

        assert_ne!(
            harness.width(children[0]),
            first,
            "`Ticks::next()` was taken for a constant and kept from mount"
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
        assert_eq!(
            after[0], before[2],
            "a keyed template moved rather than remounted"
        );
        assert_eq!(after[1], before[0]);
        assert!(
            !harness.ui().contains(before[1]),
            "the dropped row was removed"
        );
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
        assert_eq!(
            UNMOUNTED.with(Cell::get),
            unmounted + 1,
            "the component did not unmount"
        );
    }

    #[test]
    fn expanding_a_template_gives_the_tree_it_stands_for() {
        let element = xui! {
            <column gap={4.0}>
                <text>{"a"}</text>
                <container on_click={|_, _| EventResult::Consumed} />
            </column>
        };
        let crate::element::Expanded::Host(column) = element.expand_one() else {
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

    fn constant_component(cx: &mut HookContext<'_>) -> ElementDesc {
        let count = cx.use_state(|| 1u32);
        COUNT.with(|slot| *slot.borrow_mut() = Some(count));
        let width = *count.get() as f32;
        xui! {
            <row width={width}>
                <swapping flag={true} />
                <swapping flag={width > 1.0} />
            </row>
        }
    }

    #[test]
    fn a_constant_component_is_not_rendered_again() {
        let mut harness = Harness::new(constant_component);
        let renders = CHILD_RENDERS.with(Cell::get);

        set(&COUNT, 2);
        harness.render();

        assert_eq!(
            CHILD_RENDERS.with(Cell::get) - renders,
            1,
            "only the component given a dynamic prop renders again"
        );
        assert_eq!(texts(&harness, harness.top()), ["swapped", "swapped"]);
    }

    const ROWS: usize = 200;

    fn template_page(cx: &mut HookContext<'_>) -> ElementDesc {
        let count = cx.use_state(|| 1u32);
        COUNT.with(|slot| *slot.borrow_mut() = Some(count));
        let width = *count.get() as f32;
        let rows = (0..ROWS)
            .map(|_| {
                xui! {
                    <row height={20.0} gap={4.0} background={Color::BLACK}>
                        <text>{"label"}</text>
                        <container width={16.0} height={16.0} background={Color::WHITE} />
                    </row>
                }
            })
            .collect::<Vec<_>>();
        xui! {
            <column>
                <container width={width} height={4.0} />
                <column>{rows}</column>
            </column>
        }
    }

    fn draft_page(cx: &mut HookContext<'_>) -> ElementDesc {
        let count = cx.use_state(|| 1u32);
        COUNT.with(|slot| *slot.borrow_mut() = Some(count));
        let width = *count.get() as f32;
        let rows = (0..ROWS)
            .map(|_| {
                row()
                    .with::<container::height>(20.0)
                    .with::<container::gap>(4.0)
                    .with::<container::background>(Color::BLACK)
                    .into_element_desc(vec![
                        text()
                            .with::<text::content>("label")
                            .into_element_desc(Vec::new()),
                        container()
                            .with::<container::width>(16.0)
                            .with::<container::height>(16.0)
                            .with::<container::background>(Color::WHITE)
                            .into_element_desc(Vec::new()),
                    ])
            })
            .collect::<Vec<_>>();
        column().into_element_desc(vec![
            container()
                .with::<container::width>(width)
                .with::<container::height>(4.0)
                .into_element_desc(Vec::new()),
            column().into_element_desc(rows),
        ])
    }

    /// Benchmark, not a gate: run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn template_updates_against_drafts() {
        for (name, root) in [
            ("template", template_page as crate::app::ComponentFn),
            ("drafts", draft_page),
        ] {
            let mut harness = Harness::new(root);
            let updates = harness.host_updates();
            let started = std::time::Instant::now();
            const FRAMES: u32 = 50;
            for frame in 0..FRAMES {
                set(&COUNT, frame + 2);
                harness.render();
            }
            let elapsed = started.elapsed() / FRAMES;
            eprintln!(
                "{name}: {elapsed:?} per update, {} host updates per update",
                (harness.host_updates() - updates) / FRAMES as usize
            );
        }
    }
}
