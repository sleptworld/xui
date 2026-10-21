//! Only the minimum is universal: a capability owns a configuration block.
//! Style, event handling, focus management, etc. are opt-in extension traits.
use std::marker::PhantomData;
use crate::core::{Dirty, ParentStyle, StateMatcher, Theme, WidgetState};

pub trait Capability {
    type Config: Default;
}

/// Implemented only for *selected* capability blocks, on a generated CapSet storage.
pub trait Has<C: Capability> {
    fn get(&self) -> &C::Config;
    fn get_mut(&mut self) -> &mut C::Config;
}

/// The actual physical storage of a widget's selected capabilities.
pub trait CapSet {
    type Config: Default;
    /// Only selected capabilities have fields in this generated storage.
    type RuntimeState: Default;
    /// Only managed properties add fields to this generated storage.
    type ManagedState: Default;

    /// Handwritten equivalent of the `#[capability]`/`#[widget]` generated
    /// commit list. This is called once per committed mount/update/unmount,
    /// not once for every Slot::apply during a tentative render.
    fn commit_managed(
        _old: Option<&Self::Config>,
        _new: Option<&Self::Config>,
        _state: &mut Self::ManagedState,
        _ctx: &mut crate::runtime::CommitCtx<'_>,
    ) {}
}

/// Per-capability runtime local state access; implementation exists only when
/// the capability is selected by the widget's CapSet.
pub trait HasRuntime<C: InteractionCapability> {
    fn get_runtime(&self) -> &C::LocalState;
    fn get_runtime_mut(&mut self) -> &mut C::LocalState;
}

/// Style-specific merge rule. `Unset` in a source patch means "no override".
/// Ordinary Property::replace does NOT have this behavior.
pub trait StyleOverlay: Clone + Default {
    fn overlay(&mut self, source: &Self);
}

impl StyleOverlay for () {
    fn overlay(&mut self, _: &Self) {}
}

/// A capability only implements this when it participates in style resolution.
/// No interaction, focus, or keyboard capability is forced to implement it.
pub trait StyleCapability: Capability {
    type Computed: Clone + PartialEq;

    fn resolve(config: &Self::Config, parent: &ParentStyle, theme: &Theme) -> Self::Computed;

    fn propagate(_value: &Self::Computed, _child: &mut ParentStyle) {}
}

#[derive(Clone, Debug)]
pub struct StateRule<P> {
    pub matcher: StateMatcher,
    pub patch: P,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Resolved<T> {
    pub computed: T,
    pub for_children: ParentStyle,
}

impl<T> Resolved<T> {
    pub fn child_inheritance_changed(&self, next: &Self) -> bool {
        self.for_children.color != next.for_children.color
            || self.for_children.font_size != next.for_children.font_size
    }
}

/// Generated per CapSet: the Patch/Computed types ONLY include style capabilities.
/// A Pointer capability can coexist with Layout without appearing here at all.
pub trait StyleSet: CapSet {
    type Patch: StyleOverlay;
    type Computed: Clone + PartialEq;

    fn resolve_style(
        config: &Self::Config,
        rules: &[StateRule<Self::Patch>],
        state: WidgetState,
        parent: &ParentStyle,
        theme: &Theme,
    ) -> Resolved<Self::Computed>;

    fn diff(before: &Self::Computed, after: &Self::Computed) -> Dirty;
}

/// State-dependent style declarations only; **not** an overlay over interaction.
/// No RefCell and no runtime cache inside the potentially shareable declaration.
pub struct Style<C: StyleSet> {
    rules: Vec<StateRule<C::Patch>>,
    _capset: PhantomData<fn() -> C>,
}

impl<C: StyleSet> Default for Style<C> {
    fn default() -> Self {
        Self { rules: Vec::new(), _capset: PhantomData }
    }
}

impl<C: StyleSet> Style<C> {
    pub fn new() -> Self { Self::default() }

    pub fn when(&mut self, matcher: StateMatcher, patch: C::Patch) {
        self.rules.push(StateRule { matcher, patch });
    }

    pub fn rules(&self) -> &[StateRule<C::Patch>] { &self.rules }

    pub fn resolve(
        &self,
        config: &C::Config,
        state: WidgetState,
        parent: &ParentStyle,
        theme: &Theme,
    ) -> Resolved<C::Computed> {
        C::resolve_style(config, &self.rules, state, parent, theme)
    }
}

pub fn diff_resolved<C: StyleSet>(
    before: &Resolved<C::Computed>, after: &Resolved<C::Computed>,
) -> Dirty {
    let mut flags = C::diff(&before.computed, &after.computed);
    if before.child_inheritance_changed(after) { flags |= Dirty::INHERITED; }
    flags
}

/// The event router decides *which* node gets an event. The capability handles
/// that routed event, without owning global hit testing or pointer capture.
pub trait InteractionCapability: Capability {
    type LocalState: Default;
    fn on_routed_event(
        config: &Self::Config,
        local: &mut Self::LocalState,
        event: &crate::interaction::UiEvent,
    ) -> crate::interaction::EventOutcome;
}

/// Focus itself is globally owned by FocusManager, not every widget instance.
pub trait FocusCapability: Capability {}
