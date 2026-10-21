//! Every capability a node can select — [`style`] groups of style
//! properties and [`behavior`] for everything else — and the registry that
//! gives the runtime one column per capability.
//!
//! The registry is the only list of capabilities. From it come each one's
//! [`CapId`], the [`CapMask`] a node definition selects, and the runtime's
//! [`Columns`]: one map from node to storage per capability, plus one per
//! style capability for its state layers. A node's data is spread over the
//! columns of the capabilities it selected; a capability it did not select
//! has no row, rather than an empty one. The runtime reads a capability by
//! reading its column, so a subsystem that cares about focus visits exactly
//! the nodes that have it.

// Nothing in the runtime reads the columns until the widgets move onto them.
#![allow(dead_code)]

pub mod behavior;
pub mod style;

use slotmap::{SecondaryMap, SparseSecondaryMap};
use smallvec::SmallVec;
use xui_interface::style::{ComputedStyle, Theme, WidgetStateMatcher};
use xui_interface::{Animation, NodeId, Transition, WidgetState, WidgetUpdateFlags};

use crate::capability::{CommitCapability, CommitCx, HostCapability, StyleCapability};

/// A capability's place in the registry, and its column.
///
/// The column functions are for the runtime and for what `#[widget]`
/// generates; they are public only because generated code names them.
pub trait Registered: HostCapability {
    const ID: CapId;

    #[doc(hidden)]
    fn get(columns: &Columns, id: NodeId) -> Option<&Self::Storage>;

    /// Writes `value` as `id`'s row and returns what that dirtied: empty when
    /// the row already held it, [`HostCapability::DIRTY`] otherwise. A
    /// [`CommitCapability`] commits the change.
    #[doc(hidden)]
    fn write(
        columns: &mut Columns,
        id: NodeId,
        value: Self::Storage,
        cx: &mut CommitCx<'_>,
    ) -> WidgetUpdateFlags;

    #[doc(hidden)]
    fn iter(columns: &Columns) -> impl Iterator<Item = (NodeId, &Self::Storage)>;
}

/// A registered style capability, which also has a column of state layers.
pub trait RegisteredStyle: Registered + StyleCapability {
    #[doc(hidden)]
    fn layers(columns: &Columns, id: NodeId) -> &[(WidgetStateMatcher, Self::Storage)];

    /// Replaces `id`'s layers; returns whether they changed.
    #[doc(hidden)]
    fn write_layers(columns: &mut Columns, id: NodeId, layers: Layers<Self::Storage>) -> bool;

    /// `id`'s two keyframes for this capability, when its animation names
    /// any of the capability's properties.
    #[doc(hidden)]
    fn keyframes(columns: &Columns, id: NodeId) -> Option<&(Self::Storage, Self::Storage)>;

    /// Replaces `id`'s keyframes; returns whether they changed.
    #[doc(hidden)]
    fn write_keyframes(
        columns: &mut Columns,
        id: NodeId,
        keyframes: Option<(Self::Storage, Self::Storage)>,
    ) -> bool;
}

/// Which keyframe of an animation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Keyframe {
    From,
    To,
}

/// A set of capabilities: what a node definition selects.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct CapMask(u64);

impl CapMask {
    pub const EMPTY: Self = Self(0);

    pub const fn of<C: Registered>() -> Self {
        Self(1 << C::ID as u8)
    }

    pub const fn with<C: Registered>(self) -> Self {
        Self(self.0 | Self::of::<C>().0)
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn contains<C: Registered>(self) -> bool {
        self.0 & Self::of::<C>().0 != 0
    }

    /// Whether the two sets share a capability.
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// The state layers one style capability declares on one node, in rule
/// order. A layer that leaves the capability entirely unset is not stored.
pub type Layers<S> = SmallVec<[(WidgetStateMatcher, S); 1]>;

macro_rules! capabilities {
    (
        style { $($sfield:ident : $scap:ident [$skind:ident]),* $(,)? }
        behavior { $($bfield:ident : $bcap:ident [$bkind:ident $($bcommit:ident)?]),* $(,)? }
    ) => {
        /// Every capability, numbered in registry order.
        ///
        /// Variants take the capability's module name: a variant named like
        /// the marker would make the marker's name ambiguous, and rustc would
        /// print every capability by its full path in diagnostics.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(u8)]
        #[allow(non_camel_case_types)]
        pub enum CapId {
            $($sfield,)*
            $($bfield,)*
        }


        impl CapMask {
            /// The capabilities whose computed style the runtime paints as a
            /// node's decoration, under its content.
            pub const DECORATION: Self = Self::EMPTY
                .with::<style::Background>()
                .with::<style::CornerRadius>()
                .with::<style::Border>()
                .with::<style::BoxShadow>();

            /// Every style capability.
            pub const STYLE: Self = Self::EMPTY $(.with::<style::$scap>())*;
        }

        /// Every capability's property table, imported unnamed: what the
        /// element macros glob-import so that a node's table answers for the
        /// capabilities it selected.
        #[doc(hidden)]
        pub mod tables {
            $(pub use super::style::$sfield::Properties as _;)*
            $(pub use super::behavior::$bfield::Properties as _;)*
        }

        /// One column per capability, and one per style capability for its
        /// state layers.
        #[doc(hidden)]
        #[derive(Default)]
        pub struct Columns {
            $($sfield: capabilities!(@map $skind style::$sfield::Storage),)*
            $($bfield: capabilities!(@map $bkind behavior::$bfield::Storage),)*
            layers: StyleLayers,
            keyframes: StyleKeyframes,
        }

        #[derive(Default)]
        struct StyleLayers {
            $($sfield: SparseSecondaryMap<NodeId, Layers<style::$sfield::Storage>>,)*
        }

        #[derive(Default)]
        struct StyleKeyframes {
            $($sfield: SparseSecondaryMap<NodeId, (style::$sfield::Storage, style::$sfield::Storage)>,)*
        }

        $(
            impl Registered for style::$scap {
                const ID: CapId = CapId::$sfield;

                fn get(columns: &Columns, id: NodeId) -> Option<&style::$sfield::Storage> {
                    columns.$sfield.get(id)
                }

                fn write(
                    columns: &mut Columns,
                    id: NodeId,
                    value: style::$sfield::Storage,
                    _: &mut CommitCx<'_>,
                ) -> WidgetUpdateFlags {
                    if columns.$sfield.get(id) == Some(&value) {
                        return WidgetUpdateFlags::empty();
                    }
                    let old = columns.$sfield.insert(id, value);
                    capabilities!(@dirty Self, old, columns.$sfield.get(id))
                }

                fn iter(columns: &Columns) -> impl Iterator<Item = (NodeId, &style::$sfield::Storage)> {
                    columns.$sfield.iter()
                }
            }

            impl RegisteredStyle for style::$scap {
                fn layers(
                    columns: &Columns,
                    id: NodeId,
                ) -> &[(WidgetStateMatcher, style::$sfield::Storage)] {
                    columns.layers.$sfield.get(id).map_or(&[], |layers| layers.as_slice())
                }

                fn write_layers(
                    columns: &mut Columns,
                    id: NodeId,
                    layers: Layers<style::$sfield::Storage>,
                ) -> bool {
                    let old = columns.layers.$sfield.get(id).map_or(&[][..], |old| old.as_slice());
                    if old == layers.as_slice() {
                        return false;
                    }
                    if layers.is_empty() {
                        columns.layers.$sfield.remove(id);
                    } else {
                        columns.layers.$sfield.insert(id, layers);
                    }
                    true
                }

                fn keyframes(
                    columns: &Columns,
                    id: NodeId,
                ) -> Option<&(style::$sfield::Storage, style::$sfield::Storage)> {
                    columns.keyframes.$sfield.get(id)
                }

                fn write_keyframes(
                    columns: &mut Columns,
                    id: NodeId,
                    keyframes: Option<(style::$sfield::Storage, style::$sfield::Storage)>,
                ) -> bool {
                    if columns.keyframes.$sfield.get(id) == keyframes.as_ref() {
                        return false;
                    }
                    match keyframes {
                        Some(keyframes) => columns.keyframes.$sfield.insert(id, keyframes),
                        None => columns.keyframes.$sfield.remove(id),
                    };
                    true
                }
            }
        )*

        $(
            impl Registered for behavior::$bcap {
                const ID: CapId = CapId::$bfield;

                fn get(columns: &Columns, id: NodeId) -> Option<&behavior::$bfield::Storage> {
                    columns.$bfield.get(id)
                }

                /// A default row is not stored: a node that sets nothing of
                /// a behaviour has no row for it, so a column holds only
                /// the nodes that use it.
                #[allow(unused_variables)]
                fn write(
                    columns: &mut Columns,
                    id: NodeId,
                    value: behavior::$bfield::Storage,
                    cx: &mut CommitCx<'_>,
                ) -> WidgetUpdateFlags {
                    let unset = value == behavior::$bfield::Storage::default();
                    if columns.$bfield.get(id) == Some(&value)
                        || (unset && columns.$bfield.get(id).is_none())
                    {
                        return WidgetUpdateFlags::empty();
                    }
                    if unset {
                        let old = columns.$bfield.remove(id);
                        capabilities!(
                            @commit [$($bcommit)?] behavior::$bcap, id, old.as_ref(), None, cx
                        );
                        return capabilities!(@dirty Self, old, Some(&value));
                    }
                    let old = columns.$bfield.insert(id, value);
                    capabilities!(
                        @commit [$($bcommit)?] behavior::$bcap, id, old.as_ref(), columns.$bfield.get(id), cx
                    );
                    capabilities!(@dirty Self, old, columns.$bfield.get(id))
                }

                fn iter(columns: &Columns) -> impl Iterator<Item = (NodeId, &behavior::$bfield::Storage)> {
                    columns.$bfield.iter()
                }
            }
        )*

        impl Columns {
            /// Drops every row `id` has under `caps`, committing the
            /// unmount of each capability that binds something.
            pub fn remove(&mut self, id: NodeId, caps: CapMask, cx: &mut CommitCx<'_>) {
                $(
                    if caps.contains::<style::$scap>() {
                        self.$sfield.remove(id);
                        self.layers.$sfield.remove(id);
                        self.keyframes.$sfield.remove(id);
                    }
                )*
                $(
                    if caps.contains::<behavior::$bcap>() {
                        #[allow(unused_variables)]
                        let old = self.$bfield.remove(id);
                        capabilities!(@commit [$($bcommit)?] behavior::$bcap, id, old.as_ref(), None, cx);
                    }
                )*
            }

            /// Whether moving `id` from `before` to `after` starts or stops
            /// one of its state layers.
            pub(crate) fn affects_state_change(
                &self,
                id: NodeId,
                caps: CapMask,
                before: WidgetState,
                after: WidgetState,
            ) -> bool {
                before != after
                    && (false
                        $(
                            || (caps.contains::<style::$scap>()
                                && <style::$scap as RegisteredStyle>::layers(self, id)
                                    .iter()
                                    .any(|(matcher, _)| matcher.matches_state_change(before, after)))
                        )*)
            }

            /// Resolves every style capability `id` selected, in `state`,
            /// into `target`.
            #[allow(clippy::too_many_arguments)]
            pub(crate) fn apply_style(
                &self,
                id: NodeId,
                caps: CapMask,
                state: WidgetState,
                target: &mut ComputedStyle,
                parent: &ComputedStyle,
                initial: &ComputedStyle,
                theme: &Theme,
            ) {
                $(
                    if caps.contains::<style::$scap>() {
                        with_effective::<style::$scap, _>(self, id, state, |style| {
                            style::$scap::apply(style, target, parent, initial, theme)
                        });
                    }
                )*
            }

            /// `base` with one of `id`'s keyframes resolved over it, for
            /// every style capability its animation names; equal to `base`
            /// everywhere else, which keeps the layers below showing there.
            pub(crate) fn apply_keyframe(
                &self,
                id: NodeId,
                caps: CapMask,
                keyframe: Keyframe,
                base: &mut ComputedStyle,
                parent: &ComputedStyle,
                initial: &ComputedStyle,
                theme: &Theme,
            ) {
                $(
                    if caps.contains::<style::$scap>()
                        && let Some((from, to)) = <style::$scap as RegisteredStyle>::keyframes(self, id)
                    {
                        let frame = match keyframe {
                            Keyframe::From => from,
                            Keyframe::To => to,
                        };
                        style::$scap::apply(frame, base, parent, initial, theme);
                    }
                )*
            }

            pub(crate) fn inherits_differently(
                &self,
                id: NodeId,
                caps: CapMask,
                state: WidgetState,
                current: &ComputedStyle,
                parent: &ComputedStyle,
            ) -> bool {
                false
                    $(
                        || (caps.contains::<style::$scap>()
                            && with_effective::<style::$scap, _>(self, id, state, |style| {
                                style::$scap::inherits_differently(style, current, parent)
                            }))
                    )*
            }

            pub(crate) fn inherit_unset(
                &self,
                id: NodeId,
                caps: CapMask,
                state: WidgetState,
                target: &mut ComputedStyle,
                parent: &ComputedStyle,
            ) {
                $(
                    if caps.contains::<style::$scap>() {
                        with_effective::<style::$scap, _>(self, id, state, |style| {
                            style::$scap::inherit_unset(style, target, parent)
                        });
                    }
                )*
            }
        }
    };

    (@dirty $cap:ty, $old:expr, $new:expr) => {
        match ($old, $new) {
            (Some(old), Some(new)) => <$cap as HostCapability>::dirty(&old, new),
            _ => <$cap as HostCapability>::DIRTY,
        }
    };

    (@map dense $storage:ty) => { SecondaryMap<NodeId, $storage> };
    (@map sparse $storage:ty) => { SparseSecondaryMap<NodeId, $storage> };

    (@commit [] $cap:ty, $id:expr, $old:expr, $new:expr, $cx:expr) => {};
    (@commit [commit] $cap:ty, $id:expr, $old:expr, $new:expr, $cx:expr) => {
        <$cap as CommitCapability>::commit($id, $old, $new, $cx)
    };
}

// `dense` columns are a slot per node (`SecondaryMap`): for what most nodes
// select. `sparse` columns hold only the nodes that have the row
// (`SparseSecondaryMap`): for what few nodes select, or what is large.
capabilities! {
    style {
        box_size: BoxSize [dense],
        margin: Margin [sparse],
        padding: Padding [dense],
        flex_layout: FlexLayout [dense],
        position: Position [sparse],
        display: Display [sparse],
        typography: Typography [dense],
        background: Background [dense],
        corner_radius: CornerRadius [dense],
        border: Border [sparse],
        box_shadow: BoxShadow [sparse],
        clip: Clip [sparse],
        visibility: Visibility [sparse],
        transform: Transform [sparse],
        effects: Effects [sparse],
        scroll: Scroll [sparse],
        cursor: Cursor [sparse],
    }
    behavior {
        focus: Focus [sparse],
        accessibility: Accessibility [sparse],
        shortcuts: Shortcuts [sparse],
        motion: Motion [sparse],
        focus_binding: FocusBinding [sparse commit],
        scroll_binding: ScrollBinding [sparse commit],
        size_observation: SizeObservation [sparse commit],
        events: Events [sparse],
    }
}

/// The base row of `C` for `id` with every layer matching `state` merged
/// over it, given to `f` without a clone when no layer matches.
fn with_effective<C: RegisteredStyle, R>(
    columns: &Columns,
    id: NodeId,
    state: WidgetState,
    f: impl FnOnce(&C::Storage) -> R,
) -> R {
    let base = C::get(columns, id).expect("a node has a row for every capability it selects");
    let mut matching = C::layers(columns, id)
        .iter()
        .filter(|(matcher, _)| matcher.matches(state));
    match matching.next() {
        None => f(base),
        Some((_, first)) => {
            let mut effective = base.clone();
            C::merge(&mut effective, first);
            for (_, layer) in matching {
                C::merge(&mut effective, layer);
            }
            f(&effective)
        }
    }
}

impl Columns {
    /// `id`'s computed style in `state`, under `parent`: what it inherits,
    /// with every style capability it selected resolved over that.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn compute_style(
        &self,
        id: NodeId,
        caps: CapMask,
        state: WidgetState,
        parent: &ComputedStyle,
        initial: &ComputedStyle,
        theme: &Theme,
    ) -> ComputedStyle {
        let mut computed = parent.inherited_from(theme);
        self.apply_style(id, caps, state, &mut computed, parent, initial, theme);
        computed
    }

    /// The two styles `id`'s keyframe animation plays between, over `base`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn keyframe_styles(
        &self,
        id: NodeId,
        caps: CapMask,
        base: &ComputedStyle,
        parent: &ComputedStyle,
        initial: &ComputedStyle,
        theme: &Theme,
    ) -> (ComputedStyle, ComputedStyle) {
        let mut from = base.clone();
        self.apply_keyframe(id, caps, Keyframe::From, &mut from, parent, initial, theme);
        let mut to = base.clone();
        self.apply_keyframe(id, caps, Keyframe::To, &mut to, parent, initial, theme);
        (from, to)
    }

    /// `id`'s transition, when it selected `motion`.
    pub(crate) fn transition(&self, id: NodeId) -> Option<Transition> {
        behavior::Motion::get(self, id).and_then(|motion| motion.transition)
    }

    /// `id`'s keyframe animation timing, when it selected `motion`.
    pub(crate) fn animation(&self, id: NodeId) -> Option<Animation> {
        behavior::Motion::get(self, id).and_then(|motion| motion.animation)
    }
}

#[cfg(test)]
mod tests {
    use slotmap::SlotMap;
    use xui_interface::style::WidgetStateMatcher;
    use xui_interface::{Color, Transition, WidgetState};
    use xui_macros::widget;

    use super::*;
    use crate::focus::FocusHandle;
    use crate::node::{Draft, HostNode, Mounted, Node};
    use crate::scroll::{ScrollController, ScrollRequestQueue};
    use crate::capability::StyleSet;

    fn ids(n: usize) -> Vec<NodeId> {
        let mut map = SlotMap::<NodeId, ()>::with_key();
        (0..n).map(|_| map.insert(())).collect()
    }

    struct Cx {
        scroll_requests: ScrollRequestQueue,
        detached: Vec<crate::size::SizeObserver>,
    }

    impl Cx {
        fn new() -> Self {
            Self {
                scroll_requests: ScrollRequestQueue::default(),
                detached: Vec::new(),
            }
        }
        fn cx(&mut self) -> CommitCx<'_> {
            CommitCx {
                scroll_requests: &self.scroll_requests,
                detached_size_observers: &mut self.detached,
            }
        }
    }

    #[test]
    fn masks_follow_the_registry() {
        let mask = CapMask::EMPTY
            .with::<style::BoxSize>()
            .with::<behavior::Focus>();
        assert!(mask.contains::<style::BoxSize>());
        assert!(mask.contains::<behavior::Focus>());
        assert!(!mask.contains::<style::Padding>());
        assert!(CapMask::STYLE.contains::<style::Cursor>());
        assert!(!CapMask::STYLE.contains::<behavior::Motion>());
        assert_eq!(CapId::box_size as u8, 0);
    }

    #[test]
    fn writes_report_dirt_only_when_a_row_changes() {
        let [id] = ids(1)[..] else { unreachable!() };
        let mut columns = Columns::default();
        let mut cx = Cx::new();

        let focus = xui_interface::FocusProperties::default().tab_index(2);
        assert_eq!(
            behavior::Focus::write(&mut columns, id, focus, &mut cx.cx()),
            WidgetUpdateFlags::STATE_CHANGE
        );
        assert_eq!(
            behavior::Focus::write(&mut columns, id, focus, &mut cx.cx()),
            WidgetUpdateFlags::empty()
        );
        let size = style::box_size::Storage::default();
        assert_eq!(
            style::BoxSize::write(&mut columns, id, size, &mut cx.cx()),
            WidgetUpdateFlags::STYLE_TARGET
        );
        assert_eq!(behavior::Focus::iter(&columns).count(), 1);
    }

    #[test]
    fn bindings_follow_commits_and_unmount() {
        let [id] = ids(1)[..] else { unreachable!() };
        let mut columns = Columns::default();
        let mut cx = Cx::new();
        let caps = CapMask::EMPTY
            .with::<behavior::FocusBinding>()
            .with::<behavior::ScrollBinding>();

        let first = FocusHandle::new();
        let controller = ScrollController::new();
        behavior::FocusBinding::write(
            &mut columns,
            id,
            behavior::focus_binding::Storage {
                focus_handle: Some(first.clone()),
            },
            &mut cx.cx(),
        );
        behavior::ScrollBinding::write(
            &mut columns,
            id,
            behavior::scroll_binding::Storage {
                scroll_controller: Some(controller.clone()),
            },
            &mut cx.cx(),
        );
        assert_eq!(first.node_id(), Some(id));
        assert_eq!(controller.node_id(), Some(id));

        // A different handle takes over; the old one lets go.
        let second = FocusHandle::new();
        behavior::FocusBinding::write(
            &mut columns,
            id,
            behavior::focus_binding::Storage {
                focus_handle: Some(second.clone()),
            },
            &mut cx.cx(),
        );
        assert!(!first.is_bound());
        assert_eq!(second.node_id(), Some(id));

        columns.remove(id, caps, &mut cx.cx());
        assert!(!second.is_bound());
        assert_eq!(controller.node_id(), None);
        assert!(behavior::FocusBinding::get(&columns, id).is_none());
    }

    #[widget(style(background, typography, border), behavior(motion))]
    struct Swatch {}

    const SWATCH: CapMask = CapMask::EMPTY
        .with::<style::Background>()
        .with::<style::Typography>()
        .with::<style::Border>()
        .with::<behavior::Motion>();

    fn swatch() -> Draft<Swatch> {
        Draft::new(SwatchProps::builder().build())
            .with::<swatch::background>(Color::WHITE)
            .with::<swatch::border_width>(2.0)
            .with::<swatch::transition>(Transition::default())
            .when(WidgetStateMatcher::all(WidgetState::HOVERED), |layer| {
                layer.set::<swatch::background>(Color::BLACK);
                layer.set::<swatch::color>(Color::BLACK);
            })
            .when(WidgetStateMatcher::all(WidgetState::PRESSED), |layer| {
                // Changes the border the unset stroke follows.
                layer.set::<swatch::border_color>(Color::WHITE);
            })
    }

    fn scatter(
        draft: Draft<Swatch>,
        id: NodeId,
        columns: &mut Columns,
        cx: &mut Cx,
    ) -> WidgetUpdateFlags {
        Swatch::scatter(draft, id, columns, &mut cx.cx()).1
    }

    #[test]
    fn columns_resolve_like_the_node_they_came_from() {
        let theme = Theme::default();
        let initial = ComputedStyle::initial(&theme);
        let mut parent = initial.clone();
        parent.text.color = Color::rgb(0.5, 0.5, 0.5);

        // The reference: the draft itself, its rules merged in place.
        let draft = swatch();
        let effective = |state: WidgetState| {
            let mut style = draft.storage.style.clone();
            for rule in &draft.storage.rules {
                if rule.matcher.matches(state) {
                    style.merge(&rule.style.0);
                }
            }
            style
        };
        let [id] = ids(1)[..] else { unreachable!() };
        let mut columns = Columns::default();
        scatter(swatch(), id, &mut columns, &mut Cx::new());

        // Only the layers that declare something were stored.
        assert_eq!(
            <style::Background as RegisteredStyle>::layers(&columns, id).len(),
            1
        );
        assert_eq!(
            <style::Border as RegisteredStyle>::layers(&columns, id).len(),
            1
        );

        let states = [
            WidgetState::empty(),
            WidgetState::HOVERED,
            WidgetState::PRESSED,
            WidgetState::HOVERED | WidgetState::PRESSED,
        ];
        for state in states {
            let mut computed = parent.inherited_from(&theme);
            columns.apply_style(id, SWATCH, state, &mut computed, &parent, &initial, &theme);
            assert_eq!(computed, draft.compute_style(state, &parent, &theme), "{state:?}");

            assert_eq!(
                columns.inherits_differently(id, SWATCH, state, &initial, &parent),
                effective(state).inherits_differently(&initial, &parent),
                "{state:?}"
            );
            // FOCUSED: a change no layer depends on.
            for after in states.into_iter().chain([state | WidgetState::FOCUSED]) {
                assert_eq!(
                    columns.affects_state_change(id, SWATCH, state, after),
                    state != after
                        && draft
                            .storage
                            .rules
                            .iter()
                            .any(|rule| rule.matcher.matches_state_change(state, after)),
                    "{state:?} -> {after:?}"
                );
            }
        }
        assert_eq!(columns.transition(id), draft.storage.behavior.motion.transition);
    }

    #[test]
    fn unmount_drops_every_row() {
        let [id, other] = ids(2)[..] else {
            unreachable!()
        };
        let mut columns = Columns::default();
        let mut cx = Cx::new();
        scatter(swatch(), id, &mut columns, &mut cx);
        scatter(swatch(), other, &mut columns, &mut cx);

        columns.remove(id, SWATCH, &mut cx.cx());
        assert!(style::Background::get(&columns, id).is_none());
        assert!(<style::Background as RegisteredStyle>::layers(&columns, id).is_empty());
        assert!(columns.transition(id).is_none());
        // Another node's rows are untouched.
        assert!(style::Background::get(&columns, other).is_some());
        assert_eq!(columns.transition(other), Some(Transition::default()));
    }

    #[test]
    fn the_generated_mask_is_what_the_widget_selects() {
        assert_eq!(Swatch::CAPS, SWATCH);
    }

    #[test]
    fn scattering_again_reports_only_what_changed() {
        let [id] = ids(1)[..] else { unreachable!() };
        let mut columns = Columns::default();
        let mut cx = Cx::new();
        let mount = scatter(swatch(), id, &mut columns, &mut cx);
        assert_eq!(mount, WidgetUpdateFlags::STYLE_TARGET);

        assert_eq!(
            scatter(swatch(), id, &mut columns, &mut cx),
            WidgetUpdateFlags::empty()
        );

        let recoloured = swatch().with::<swatch::background>(Color::rgb(1.0, 0.0, 0.0));
        assert_eq!(
            scatter(recoloured, id, &mut columns, &mut cx),
            WidgetUpdateFlags::STYLE_TARGET
        );

        // A change to a state layer alone is a style change too.
        let relayered = swatch().when(WidgetStateMatcher::all(WidgetState::FOCUSED), |layer| {
            layer.set::<swatch::border_width>(4.0);
        });
        assert_eq!(
            scatter(relayered, id, &mut columns, &mut cx),
            WidgetUpdateFlags::STYLE_TARGET
        );
        assert_eq!(
            <style::Border as RegisteredStyle>::layers(&columns, id).len(),
            2
        );
    }

    /// A widget with props of its own and a binding.
    #[widget(style(box_size), behavior(focus_binding))]
    struct Field {
        #[default]
        #[dirty(TEXT_SHAPE | LAYOUT_INPUT)]
        label: String,
        #[default]
        #[dirty(PAINT_OUTPUT)]
        dimmed: bool,
    }

    #[test]
    fn props_dirty_names_each_changed_prop() {
        let old = FieldProps {
            label: "a".into(),
            dimmed: false,
        };
        let relabelled = FieldProps {
            label: "b".into(),
            dimmed: false,
        };
        let both = FieldProps {
            label: "b".into(),
            dimmed: true,
        };
        assert_eq!(Field::props_dirty(&old, &old), WidgetUpdateFlags::empty());
        assert_eq!(
            Field::props_dirty(&old, &relabelled),
            WidgetUpdateFlags::TEXT_SHAPE | WidgetUpdateFlags::LAYOUT_INPUT
        );
        assert_eq!(
            Field::props_dirty(&old, &both),
            WidgetUpdateFlags::TEXT_SHAPE
                | WidgetUpdateFlags::LAYOUT_INPUT
                | WidgetUpdateFlags::PAINT_OUTPUT
        );
    }

    #[test]
    fn a_mounted_node_updates_props_rows_and_bindings() {
        let [id] = ids(1)[..] else { unreachable!() };
        let mut columns = Columns::default();
        let mut cx = Cx::new();
        let first = FocusHandle::new();
        let draft = Draft::<Field>::new(FieldProps::builder().build())
            .with::<field::focus_handle>(first.clone());
        let (mut props, _) = Field::scatter(draft, id, &mut columns, &mut cx.cx());
        assert_eq!(first.node_id(), Some(id));

        let second = FocusHandle::new();
        {
            let mut commit = cx.cx();
            let mut node = Mounted::<Field> {
                id,
                props: &mut props,
                columns: &mut columns,
                cx: &mut commit,
            };
            assert_eq!(
                node.update::<field::width>(40.0),
                WidgetUpdateFlags::STYLE_TARGET
            );
            assert_eq!(
                node.update::<field::width>(40.0),
                WidgetUpdateFlags::empty()
            );
            assert_eq!(
                node.update::<field::dimmed>(true),
                WidgetUpdateFlags::PAINT_OUTPUT
            );
            // Through `update`, a swapped handle is committed: the old one lets go.
            assert_eq!(
                node.update::<field::focus_handle>(second.clone()),
                WidgetUpdateFlags::STATE_CHANGE
            );
        }

        assert!(props.dimmed);
        assert!(!first.is_bound());
        assert_eq!(second.node_id(), Some(id));
        assert_eq!(
            style::BoxSize::get(&columns, id).map(|size| size.width),
            Some(xui_interface::styles::units::StyleValue::Value(40.0.into()))
        );
    }

    /// A widget that listens for events.
    #[widget(style(background), behavior(events))]
    struct Button {}

    fn button() -> Draft<Button> {
        // No annotations: `on` spells out the handler's `Fn` bound, so the
        // closure infers `&ClickEvent` and `&mut EventContext<'_>`, and the
        // method calls below type-check against them.
        Draft::new(ButtonProps::builder().build()).with_on::<button::on_click, _>(|event, cx| {
            let _ = (event.click_count, cx.phase);
        })
    }

    use crate::event_system::callbacks::EventMask;

    #[test]
    fn handlers_are_a_capability_row() {
        let [id] = ids(1)[..] else { unreachable!() };
        let mut columns = Columns::default();
        let mut cx = Cx::new();

        let (_, mount) = Button::scatter(button(), id, &mut columns, &mut cx.cx());
        assert!(mount.contains(WidgetUpdateFlags::STATE_CHANGE));
        let handlers = behavior::Events::get(&columns, id).unwrap();
        assert!(handlers.listens_for(EventMask::Click));
        assert!(!handlers.listens_for(EventMask::DoubleClick));

        // A re-render recreates the closure: a new row, but nothing dirty.
        let (_, again) = Button::scatter(button(), id, &mut columns, &mut cx.cx());
        assert_eq!(again, WidgetUpdateFlags::empty());

        // Listening for a new kind is a state change.
        let more = button().with_on::<button::on_double_click_capture, _>(|_, _| {});
        let (_, grown) = Button::scatter(more, id, &mut columns, &mut cx.cx());
        assert_eq!(grown, WidgetUpdateFlags::STATE_CHANGE);
        assert!(
            behavior::Events::get(&columns, id)
                .unwrap()
                .listens_for(EventMask::DoubleClick)
        );

        // A ready-made handler (a forwarded `Callback` prop, say) is a plain
        // value of the write-only property: `set`, or a slot's `write`.
        // Its event type is checked like any value's: a scroll handler is not
        // a context-menu handler.
        let on_scroll = crate::event_system::Handler::new(|_: &_, _: &mut _| {});
        let on_menu = crate::event_system::Handler::new(|_: &_, _: &mut _| {});
        let mut node = button();
        node.set::<button::on_scroll>(on_scroll);
        crate::property::Slot::<button::on_context_menu>::new().write(&mut node, on_menu);
        let (_, set) = Button::scatter(node, id, &mut columns, &mut cx.cx());
        assert_eq!(set, WidgetUpdateFlags::STATE_CHANGE);
        let handlers = behavior::Events::get(&columns, id).unwrap();
        assert!(handlers.listens_for(EventMask::Scroll | EventMask::ContextMenu));

        // A mounted node takes handlers too.
        let (mut props, _) = Button::scatter(button(), id, &mut columns, &mut cx.cx());
        let mut commit = cx.cx();
        let mut node = Mounted::<Button> {
            id,
            props: &mut props,
            columns: &mut columns,
            cx: &mut commit,
        };
        assert_eq!(
            node.on::<button::on_click, _>(|_, _| {}),
            WidgetUpdateFlags::empty()
        );
        assert_eq!(
            node.on::<button::on_scroll, _>(|_, _| {}),
            WidgetUpdateFlags::STATE_CHANGE
        );
        // `set` writes write-only properties into a mounted node's row too.
        let on_wake = crate::event_system::Handler::new(|_: &_, _: &mut _| {});
        assert_eq!(
            node.set::<button::on_wake>(on_wake),
            WidgetUpdateFlags::STATE_CHANGE
        );
    }
}
