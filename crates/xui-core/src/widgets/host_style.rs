//! What the runtime asks a host widget about its style, and nothing else.
//!
//! The style pass, inheritance sync and transitions only ever go through
//! [`HostStyle`], so a widget can answer from the monolithic [`Style`] it has
//! always carried ([`LegacyStyle`]) or from its node's selected style
//! capabilities and state rules ([`NodeStyle`]) without the runtime telling
//! the two apart. Widgets move from the first to the second one at a time.

use xui_interface::{Animation, Transition, WidgetState};

use super::Widgets;
use crate::capability::StyleSet;
use crate::node::{NodeDef, NodeStorage, StyleRule};
use crate::style::{ComputedStyle, FlexDirectionStyle, Style, Theme};

pub(crate) trait HostStyle {
    /// Whether moving from `before` to `after` can change the computed style:
    /// whether some state rule starts or stops matching.
    fn affects_state_change(&self, before: WidgetState, after: WidgetState) -> bool;

    /// The node's computed style in `state`, under `parent`.
    fn compute(&self, parent: &ComputedStyle, theme: &Theme, state: WidgetState) -> ComputedStyle;

    /// Whether [`inherit_unset`](Self::inherit_unset) would change `current`.
    fn inherits_differently(
        &self,
        state: WidgetState,
        current: &ComputedStyle,
        parent: &ComputedStyle,
    ) -> bool;

    /// Copies from `parent` each inherited property the style in `state`
    /// leaves to the parent.
    fn inherit_unset(&self, state: WidgetState, target: &mut ComputedStyle, parent: &ComputedStyle);

    fn transition(&self) -> Option<Transition>;

    fn animation(&self) -> Option<Animation>;
}

/// A widget that still carries a [`Style`]: its default style, its style, and
/// the per-widget overrides the runtime used to apply by name.
pub(crate) struct LegacyStyle<'a> {
    widget: &'a Widgets,
}

impl<'a> LegacyStyle<'a> {
    pub(crate) fn new(widget: &'a Widgets) -> Self {
        Self { widget }
    }

    fn style(&self) -> &Style {
        self.widget.style()
    }
}

impl HostStyle for LegacyStyle<'_> {
    fn affects_state_change(&self, before: WidgetState, after: WidgetState) -> bool {
        self.style().affects_state_change(before, after)
    }

    fn compute(&self, parent: &ComputedStyle, theme: &Theme, state: WidgetState) -> ComputedStyle {
        let widget = self.widget;
        let mut computed = parent.inherited_from(theme);
        if let Some(scope) = widget.style_scope() {
            computed.apply(parent, scope, theme);
        }
        widget
            .default_style()
            .with_patch_for_state(WidgetState::empty(), |patch| {
                computed.apply(parent, patch, theme)
            });
        widget
            .style()
            .with_patch_for_state(state, |patch| computed.apply(parent, patch, theme));
        match widget {
            Widgets::Container(widget) => {
                if let Some(direction) = widget.flex_direction {
                    computed.layout.flex_direction = direction;
                }
            }
            Widgets::Root(_) => computed.layout.flex_direction = FlexDirectionStyle::Column,
            _ => {}
        }
        computed
    }

    fn inherits_differently(
        &self,
        state: WidgetState,
        current: &ComputedStyle,
        parent: &ComputedStyle,
    ) -> bool {
        self.style().with_patch_for_state(state, |patch| {
            current.inherits_differently_from(parent, patch)
        })
    }

    fn inherit_unset(
        &self,
        state: WidgetState,
        target: &mut ComputedStyle,
        parent: &ComputedStyle,
    ) {
        self.style()
            .with_patch_for_state(state, |patch| target.inherit_unset_from(parent, patch));
    }

    fn transition(&self) -> Option<Transition> {
        self.style().transition_config()
    }

    fn animation(&self) -> Option<Animation> {
        self.style().animation_config().cloned()
    }
}

/// A widget built on a node definition: its selected style capabilities,
/// layered by the state rules that match.
pub(crate) struct NodeStyle<'a, N: NodeDef> {
    pub(crate) storage: &'a NodeStorage<N>,
    pub(crate) rules: &'a [StyleRule<N::Style>],
}

impl<N: NodeDef> NodeStyle<'_, N> {
    /// The base style with every rule matching `state` merged over it, given
    /// to `f` without a clone when no rule matches.
    fn with_effective<R>(&self, state: WidgetState, f: impl FnOnce(&N::Style) -> R) -> R {
        let base = &self.storage.style;
        let mut matching = self.rules.iter().filter(|rule| rule.matcher.matches(state));
        match matching.next() {
            None => f(base),
            Some(first) => {
                let mut effective = base.clone();
                effective.merge(&first.style.0);
                for rule in matching {
                    effective.merge(&rule.style.0);
                }
                f(&effective)
            }
        }
    }
}

impl<N: NodeDef> HostStyle for NodeStyle<'_, N> {
    fn affects_state_change(&self, before: WidgetState, after: WidgetState) -> bool {
        before != after
            && self
                .rules
                .iter()
                .any(|rule| rule.matcher.matches_state_change(before, after))
    }

    fn compute(&self, parent: &ComputedStyle, theme: &Theme, state: WidgetState) -> ComputedStyle {
        crate::node::compute_style(&self.storage.style, self.rules, state, parent, theme)
    }

    fn inherits_differently(
        &self,
        state: WidgetState,
        current: &ComputedStyle,
        parent: &ComputedStyle,
    ) -> bool {
        self.with_effective(state, |style| style.inherits_differently(current, parent))
    }

    fn inherit_unset(
        &self,
        state: WidgetState,
        target: &mut ComputedStyle,
        parent: &ComputedStyle,
    ) {
        self.with_effective(state, |style| style.inherit_unset(target, parent));
    }

    fn transition(&self) -> Option<Transition> {
        N::transition(self.storage)
    }

    /// Keyframe animations are not carried by nodes yet: their frames are
    /// still `StylePatch`es.
    fn animation(&self) -> Option<Animation> {
        None
    }
}

#[cfg(test)]
mod tests {
    use xui_interface::Color;
    use xui_interface::style::WidgetStateMatcher;
    use xui_macros::widget;

    use super::*;
    use crate::node::Draft;

    #[widget(style(background, typography), behavior(motion))]
    struct Swatch {}

    fn swatch() -> Draft<Swatch> {
        Draft::new(SwatchProps::builder().build())
            .with::<swatch::background>(Color::WHITE)
            .with::<swatch::transition>(Transition::default())
            .when(WidgetStateMatcher::all(WidgetState::HOVERED), |layer| {
                layer.set::<swatch::background>(Color::BLACK);
                layer.set::<swatch::color>(Color::BLACK);
            })
    }

    #[test]
    fn node_style_answers_like_its_draft() {
        let theme = Theme::default();
        let parent = ComputedStyle::initial(&theme);
        let draft = swatch();
        let style = NodeStyle {
            storage: &draft.storage,
            rules: &draft.rules,
        };

        assert!(style.affects_state_change(WidgetState::empty(), WidgetState::HOVERED));
        assert!(!style.affects_state_change(WidgetState::empty(), WidgetState::PRESSED));
        assert_eq!(
            style.compute(&parent, &theme, WidgetState::HOVERED),
            draft.compute_style(WidgetState::HOVERED, &parent, &theme)
        );
        assert_eq!(style.transition(), Some(Transition::default()));
        assert_eq!(style.animation(), None);
    }

    #[test]
    fn node_style_inherits_through_the_matching_layers() {
        let theme = Theme::default();
        let mut parent = ComputedStyle::initial(&theme);
        parent.text.color = Color::WHITE;
        let draft = swatch();
        let style = NodeStyle {
            storage: &draft.storage,
            rules: &draft.rules,
        };
        let current = ComputedStyle::initial(&theme);

        // Idle, the colour is left to the parent; hovered, the rule sets it.
        assert!(style.inherits_differently(WidgetState::empty(), &current, &parent));
        assert!(!style.inherits_differently(WidgetState::HOVERED, &current, &parent));

        let mut synced = current.clone();
        style.inherit_unset(WidgetState::empty(), &mut synced, &parent);
        assert_eq!(synced.text.color, Color::WHITE);
    }
}
