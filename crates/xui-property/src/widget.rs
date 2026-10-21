//! Handwritten `#[widget]` output and enum runtime dispatch.
//!
//! The sealed-looking enum is deliberate: no TypeId probing and no hundreds
//! of per-ability function pointers. The generator emits the selected arms.
use crate::{
    capability::{CapSet, Has, HasRuntime, Resolved, StyleSet, InteractionCapability, diff_resolved},
    core::{Dirty, ParentStyle, Theme, WidgetState},
    interaction::{EventOutcome, Interaction, UiEvent},
    nodes::{HostKind, NodeDef, NodeDraft, Text, Image, Button, Action},
    runtime::{CommitCtx, NodeId, RuntimeServices},
    property::commit_managed,
    nodes::ButtonLabel,
};

/// Host-only definition. Components implement NodeDef, but never Widget.
/// Only Widget-specific lifecycle behavior goes here. Field-level managed
/// effects are generated from the selected managed Properties on the CapSet.
pub trait Widget: NodeDef<Kind = HostKind> + Sized {
    type State: Default;
    /// Handwritten expansion of owned-Props managed resource storage.
    type ManagedProps: Default;

    fn commit_managed_props(
        _old: Option<&Self::Props>, _new: Option<&Self::Props>,
        _managed: &mut Self::ManagedProps, _ctx: &mut CommitCtx<'_>,
    ) {}

    fn on_mount(_state: &mut Self::State) {}
    fn on_update(_state: &mut Self::State, _dirty: Dirty) {}
    fn on_unmount(_state: &mut Self::State) {}
}

/// Fully constructed declarative node. `NodeDraft<W>` is already the separate
/// representation of a WIP description in this prototype.
pub type WidgetDesc<W> = NodeDraft<W>;

pub struct InstanceBox<W: Widget> {
    pub id: NodeId,
    pub description: WidgetDesc<W>,
    pub computed: Resolved<<W::Caps as StyleSet>::Computed>,
    pub state: W::State,
    pub cap_state: <W::Caps as CapSet>::RuntimeState,
    pub managed_state: <W::Caps as CapSet>::ManagedState,
    pub managed_props: W::ManagedProps,
}

impl<W: Widget> InstanceBox<W> {
    /// Only this mount changes the external registry; constructing a draft or
    /// applying a Slot to a draft does not.
    pub fn mount(
        id: NodeId,
        description: WidgetDesc<W>,
        state: WidgetState,
        parent: &ParentStyle,
        theme: &Theme,
        services: &mut RuntimeServices,
    ) -> Self {
        let computed = description.resolve_style(state, parent, theme);
        let mut result = Self {
            id, description, computed,
            state: Default::default(),
            cap_state: Default::default(),
            managed_state: Default::default(),
            managed_props: Default::default(),
        };
        let mut ctx = CommitCtx { node: id, services };
        W::Caps::commit_managed(
            None, Some(&result.description.config),
            &mut result.managed_state, &mut ctx,
        );
        W::commit_managed_props(
            None, Some(&result.description.props),
            &mut result.managed_props, &mut ctx,
        );
        W::on_mount(&mut result.state);
        result
    }

    /// Models the Fiber commit: old committed and new WIP descriptions are
    /// both visible to generated managed-property reconciliation exactly once.
    pub fn commit(
        &mut self,
        next: WidgetDesc<W>,
        hint: Dirty,
        state: WidgetState,
        parent: &ParentStyle,
        theme: &Theme,
        services: &mut RuntimeServices,
    ) -> Dirty {
        let computed_next = next.resolve_style(state, parent, theme);
        let dirty = hint | diff_resolved::<W::Caps>(&self.computed, &computed_next);
        let mut ctx = CommitCtx { node: self.id, services };
        W::Caps::commit_managed(
            Some(&self.description.config), Some(&next.config),
            &mut self.managed_state, &mut ctx,
        );
        W::commit_managed_props(
            Some(&self.description.props), Some(&next.props),
            &mut self.managed_props, &mut ctx,
        );
        self.description = next;
        self.computed = computed_next;
        if !dirty.is_empty() { W::on_update(&mut self.state, dirty); }
        dirty
    }

    /// Reconciler must call this on real unmount, not when WIP is abandoned.
    /// A future arena runtime should guarantee cleanup on tree removal.
    pub fn unmount(mut self, services: &mut RuntimeServices) {
        W::on_unmount(&mut self.state);
        let mut ctx = CommitCtx { node: self.id, services };
        W::Caps::commit_managed(
            Some(&self.description.config), None,
            &mut self.managed_state, &mut ctx,
        );
        W::commit_managed_props(
            Some(&self.description.props), None,
            &mut self.managed_props, &mut ctx,
        );
    }
}

#[derive(Default, Debug)]
pub struct TextState { pub lifecycle_updates: usize }
#[derive(Default, Debug)]
pub struct ImageState;
#[derive(Default, Debug)]
pub struct ButtonState;
#[derive(Default, Debug)]
pub struct ActionState;

impl Widget for Text {
    type State = TextState;
    type ManagedProps = ();
    fn on_update(state: &mut Self::State, _: Dirty) { state.lifecycle_updates += 1; }
}
impl Widget for Image {
    type State = ImageState;
    type ManagedProps = ();
}
impl Widget for Button {
    type State = ButtonState;
    type ManagedProps = bool;
    fn commit_managed_props(
        old: Option<&Self::Props>, new: Option<&Self::Props>,
        managed: &mut bool, ctx: &mut CommitCtx<'_>,
    ) {
        // Generated by #[widget]: ButtonLabel is a managed *owned Prop*,
        // while GlobalShortcut is a managed *capability Property*.
        commit_managed::<ButtonLabel>(old, new, managed, ctx);
    }
}
impl Widget for Action {
    type State = ActionState;
    type ManagedProps = ();
}

/// Custom content rendering is separate from Decoration, which the shared
/// scene compiler can draw without asking a Widget to implement this trait.
pub trait PaintContent: Widget {
    fn paint_content(instance: &InstanceBox<Self>, ctx: &mut PaintCtx);
}

#[derive(Default, Debug)]
pub struct PaintCtx { pub commands: Vec<String> }
impl PaintContent for Text {
    fn paint_content(instance: &InstanceBox<Self>, ctx: &mut PaintCtx) {
        ctx.commands.push(format!("text:{}", instance.description.props.content));
    }
}
impl PaintContent for Image {
    fn paint_content(instance: &InstanceBox<Self>, ctx: &mut PaintCtx) {
        ctx.commands.push(format!("image:{}", instance.description.props.source));
    }
}

/// Generic helper is compiled only for variants with Interaction.
fn routed_pointer<W: Widget>(instance: &mut InstanceBox<W>, event: &UiEvent) -> EventOutcome
where
    <W::Caps as CapSet>::Config: Has<Interaction>,
    <W::Caps as CapSet>::RuntimeState: HasRuntime<Interaction>,
{
    let config = <<W::Caps as CapSet>::Config as Has<Interaction>>::get(
        &instance.description.config,
    );
    let local = <<W::Caps as CapSet>::RuntimeState as HasRuntime<Interaction>>::get_runtime_mut(
        &mut instance.cap_state,
    );
    Interaction::on_routed_event(config, local, event)
}

/// Generated from the closed list of built-in widget types. Static arms use
/// concrete Rust types, and unsupported behavior emits no adapter call.
pub enum Widgets {
    Text(InstanceBox<Text>),
    Image(InstanceBox<Image>),
    Button(InstanceBox<Button>),
    Action(InstanceBox<Action>),
}

impl Widgets {
    pub fn supports_content_paint(&self) -> bool {
        matches!(self, Self::Text(_) | Self::Image(_))
    }
    pub fn paint_content(&self, ctx: &mut PaintCtx) {
        match self {
            Self::Text(w) => Text::paint_content(w, ctx),
            Self::Image(w) => Image::paint_content(w, ctx),
            Self::Button(_) | Self::Action(_) => {},
        }
    }
    pub fn pointer_event(&mut self, event: &UiEvent) -> Option<EventOutcome> {
        match self {
            Self::Text(w) => Some(routed_pointer(w, event)),
            Self::Button(w) => Some(routed_pointer(w, event)),
            Self::Action(w) => Some(routed_pointer(w, event)),
            Self::Image(_) => None,
        }
    }
    pub fn id(&self) -> NodeId {
        match self {
            Self::Text(w) => w.id,
            Self::Image(w) => w.id,
            Self::Button(w) => w.id,
            Self::Action(w) => w.id,
        }
    }
}
