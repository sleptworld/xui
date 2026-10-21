//! Behaviour capabilities: what a node selects besides style — focus,
//! accessibility, shortcuts, motion, and the runtime handles it binds.
//!
//! A behaviour capability has the same shape as a style one (a marker, a
//! `Storage`, a `props` module), so `#[widget(behavior(..))]` treats both
//! alike. Unlike a style capability its values are plain, not `StyleValue`
//! declarations: nothing cascades, nothing resolves against a theme, and a
//! state rule cannot reach them. Each is defined with
//! [`behavior_capability!`](crate::behavior_capability).

use crate::behavior_capability;
use crate::capability::{CommitCapability, CommitCx};
use crate::focus::FocusHandle;
use crate::scroll::ScrollController;
use crate::size::SizeObserver;
use xui_interface::{
    AccessibilityChecked, AccessibilityLiveRegion, AccessibilityOrientation, AccessibilityPopup,
    AccessibilityProperties, AccessibilityRole, FocusProperties, Focusability, NodeId,
    OrderedFloat, ShortcutBinding, Transition,
};

behavior_capability! {
    /// Whether and in which order the node takes keyboard focus.
    pub mod focus: Focus(FocusProperties) dirty(STATE_CHANGE) {
        /// `true` / `false` force focusable / not; `Focusability::Auto` keeps
        /// the widget's own behaviour.
        focusable: Focusability => focusability,
        /// Negative: focusable programmatically but skipped by Tab.
        /// Non-negative: in the tab order, positive values first.
        tab_index: i32 => tab_index [optional],
    }
}

behavior_capability! {
    /// What the node exposes to the platform accessibility tree.
    ///
    /// Every property carries the `accessibility_` prefix: unprefixed,
    /// `label` or `value` would collide with a widget's own props, and a
    /// widget's own prop silently wins over a capability's.
    pub mod accessibility: Accessibility(AccessibilityProperties, whole = accessibility)
        dirty(STATE_CHANGE)
    {
        accessibility_role: AccessibilityRole => role [optional],
        accessibility_id: String => id [optional],
        accessibility_label: String => label [optional],
        accessibility_description: String => description [optional],
        accessibility_selected: bool => selected [optional],
        accessibility_disabled: bool => disabled [optional],
        accessibility_controls: String => controls [optional],
        accessibility_labelled_by: String => labelled_by [optional],
        accessibility_described_by: String => described_by [optional],
        accessibility_value: String => value [optional],
        accessibility_read_only: bool => read_only [optional],
        accessibility_required: bool => required [optional],
        accessibility_invalid: bool => invalid [optional],
        accessibility_hidden: bool => hidden [optional],
        accessibility_expanded: bool => expanded [optional],
        accessibility_checked: AccessibilityChecked => checked [optional],
        accessibility_orientation: AccessibilityOrientation => orientation [optional],
        accessibility_live_region: AccessibilityLiveRegion => live_region [optional],
        accessibility_has_popup: AccessibilityPopup => has_popup [optional],
        accessibility_numeric_value: OrderedFloat<f64> => numeric_value [optional],
        accessibility_min_value: OrderedFloat<f64> => min_value [optional],
        accessibility_max_value: OrderedFloat<f64> => max_value [optional],
        accessibility_step_value: OrderedFloat<f64> => step_value [optional],
    }
}

behavior_capability! {
    /// Keyboard shortcuts the node binds to commands while it is mounted.
    pub mod shortcuts: Shortcuts dirty(STATE_CHANGE) {
        shortcuts: Vec<ShortcutBinding>,
    }
}

behavior_capability! {
    /// How the node's computed style moves between values.
    ///
    /// Keyframe animations are not here: their frames are layers of the
    /// node's own style type, so they live with its state rules.
    pub mod motion: Motion dirty(STYLE_TARGET) {
        transition: Transition [optional],
    }
}

/// Event handlers, one per event kind and phase.
///
/// Laid out like the other capability modules, but not defined with
/// `behavior_capability!`: its storage is the existing `EventHandlers`, and
/// its properties — write-only, one per handler slot — come from the
/// `events!` table. A closure is set through `Node::on`, which spells out
/// its `Fn` bound; a ready-made `Handler` through `set` like any value.
pub mod events {
    use crate::capability::HostCapability;
    use crate::event_system::callbacks::EventHandlers;
    use crate::property::Capability;
    use xui_interface::WidgetUpdateFlags;

    /// Event handlers, one per event kind and phase.
    pub struct Events;

    #[doc = "The storage of [`Events`]."]
    pub type Storage = EventHandlers;

    impl Capability for Events {
        type Storage = EventHandlers;
    }

    impl HostCapability for Events {
        /// Which kinds a node listens for decides whether it takes focus,
        /// starts a drag, ..
        const DIRTY: WidgetUpdateFlags = WidgetUpdateFlags::STATE_CHANGE;

        /// Handlers recreated with their closures dirty nothing; only a
        /// change to the kinds listened for does.
        fn dirty(old: &EventHandlers, new: &EventHandlers) -> WidgetUpdateFlags {
            if old.mask() == new.mask() {
                WidgetUpdateFlags::empty()
            } else {
                Self::DIRTY
            }
        }
    }

    /// One property per handler slot, named like its attribute.
    pub mod props {
        pub use crate::event_system::callbacks::handler_props::*;
    }
}

pub use events::Events;

// The three handle capabilities are named after the binding, not the
// handle: by the module-to-marker convention `focus_handle` would declare a
// marker `FocusHandle`, shadowing the handle type it stores.

behavior_capability! {
    /// A [`FocusHandle`] bound to this node while it is mounted, through which
    /// code elsewhere moves focus to it.
    pub mod focus_binding: FocusBinding dirty(STATE_CHANGE) {
        focus_handle: FocusHandle [optional],
    }
}

behavior_capability! {
    /// A [`ScrollController`] bound to this node's scroll position. Only has an
    /// effect on a node whose style makes it scrollable.
    pub mod scroll_binding: ScrollBinding dirty(STATE_CHANGE) {
        scroll_controller: ScrollController [optional],
    }
}

behavior_capability! {
    /// A [`SizeObserver`] told this node's laid-out border-box size, without a
    /// wrapper or a change to layout. Create it with `cx.use_size()`.
    pub mod size_observation: SizeObservation dirty(STATE_CHANGE) {
        size_observer: SizeObserver [optional],
    }
}

// What each binding does at commit, as the interaction system did it when it
// copied them out of `InteractionProperties`.

impl CommitCapability for FocusBinding {
    fn commit(
        node: NodeId,
        old: Option<&focus_binding::Storage>,
        new: Option<&focus_binding::Storage>,
        _: &mut CommitCx<'_>,
    ) {
        if let Some(handle) = old.and_then(|old| old.focus_handle.as_ref()) {
            handle.unbind(node);
        }
        if let Some(handle) = new.and_then(|new| new.focus_handle.as_ref()) {
            handle.bind(node);
        }
    }
}

impl CommitCapability for ScrollBinding {
    fn commit(
        node: NodeId,
        old: Option<&scroll_binding::Storage>,
        new: Option<&scroll_binding::Storage>,
        cx: &mut CommitCx<'_>,
    ) {
        if let Some(controller) = old.and_then(|old| old.scroll_controller.as_ref()) {
            controller.unbind(node);
        }
        if let Some(controller) = new.and_then(|new| new.scroll_controller.as_ref()) {
            controller.bind(node, cx.scroll_requests.clone());
        }
    }
}

impl CommitCapability for SizeObservation {
    /// Only a different observer detaches the old one: recommitting the same
    /// subscription must keep its last size.
    fn commit(
        node: NodeId,
        old: Option<&size_observation::Storage>,
        new: Option<&size_observation::Storage>,
        cx: &mut CommitCx<'_>,
    ) {
        let old = old.and_then(|old| old.size_observer.as_ref());
        let new = new.and_then(|new| new.size_observer.as_ref());
        if old != new
            && let Some(observer) = old
            && observer.unbind(node)
        {
            cx.detached_size_observers.push(observer.clone());
        }
        if let Some(observer) = new {
            observer.bind(node);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::property::{Access, CapScope, Capability, Has, Slot};
    use xui_interface::{CommandId, PhysicalKey, Shortcut, WidgetUpdateFlags};

    #[derive(Default)]
    struct Node {
        focus: FocusProperties,
        accessibility: AccessibilityProperties,
        shortcuts: shortcuts::Storage,
        motion: motion::Storage,
    }

    macro_rules! has {
        ($cap:ty, $field:ident) => {
            impl Has<$cap> for Node {
                fn get(&self) -> &<$cap as Capability>::Storage {
                    &self.$field
                }
                fn get_mut(&mut self) -> &mut <$cap as Capability>::Storage {
                    &mut self.$field
                }
            }
        };
    }
    has!(Focus, focus);
    has!(Accessibility, accessibility);
    has!(Shortcuts, shortcuts);
    has!(Motion, motion);

    impl<C: Capability> Access<CapScope<C>> for Node
    where
        Node: Has<C>,
    {
        fn access(&self) -> &C::Storage {
            Has::<C>::get(self)
        }
        fn access_mut(&mut self) -> &mut C::Storage {
            Has::<C>::get_mut(self)
        }
    }

    #[test]
    fn existing_storage_is_written_in_place() {
        use focus::props::{focusable, tab_index};

        let mut node = Node::default();
        assert_eq!(
            Slot::<focusable>::new().update(&mut node, false),
            WidgetUpdateFlags::STATE_CHANGE
        );
        assert!(Slot::<tab_index>::new().apply(&mut node, -1));
        assert_eq!(
            node.focus,
            FocusProperties {
                focusability: Focusability::NotFocusable,
                tab_index: Some(-1),
            }
        );
        assert!(Slot::<tab_index>::new().apply(&mut node, None));
        assert_eq!(node.focus.tab_index, None);
    }

    #[test]
    fn optional_properties_take_values_options_and_text() {
        use self::accessibility::props as a11y;

        let mut node = Node::default();
        Slot::<a11y::accessibility_label>::new().apply(&mut node, "Save");
        Slot::<a11y::accessibility_role>::new().apply(&mut node, AccessibilityRole::Slider);
        Slot::<a11y::accessibility_numeric_value>::new().apply(&mut node, 0.5);
        Slot::<a11y::accessibility_disabled>::new().apply(&mut node, true);
        assert_eq!(node.accessibility.label.as_deref(), Some("Save"));
        assert_eq!(node.accessibility.role, Some(AccessibilityRole::Slider));
        assert_eq!(node.accessibility.numeric_value, Some(OrderedFloat(0.5)));
        assert_eq!(node.accessibility.disabled, Some(true));

        // The whole-storage property replaces everything at once.
        let whole = AccessibilityProperties::default().role(AccessibilityRole::Dialog);
        assert_eq!(
            Slot::<a11y::accessibility>::new().update(&mut node, whole.clone()),
            WidgetUpdateFlags::STATE_CHANGE
        );
        assert_eq!(node.accessibility, whole);
        assert!(!Slot::<a11y::accessibility_label>::new().apply(&mut node, None));
    }

    #[test]
    fn generated_storage_and_motion() {
        let mut node = Node::default();
        let binding = ShortcutBinding {
            shortcut: Shortcut::physical(PhysicalKey::KeyS),
            command: CommandId("save"),
        };
        assert!(Slot::<shortcuts::props::shortcuts>::new().apply(&mut node, [binding]));
        assert_eq!(node.shortcuts.shortcuts, vec![binding]);

        let transition = Transition::default();
        assert_eq!(
            Slot::<motion::props::transition>::new().update(&mut node, transition),
            WidgetUpdateFlags::STYLE_TARGET
        );
        assert_eq!(node.motion.transition, Some(transition));
    }
}
