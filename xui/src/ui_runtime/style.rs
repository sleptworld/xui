use crate::animation::{has_animatable_difference, interpolate_style};
use slotmap::{SecondaryMap, SparseSecondaryMap};
use std::time::Duration;
use xui_animation::{Timeline, Transition};
use xui_interface::{ComputedStyle, NodeId, StyleDiffFlags, StylePatch, StyleValue, Theme};

pub(crate) struct StyleNode {
    computed: ComputedStyle,
    initialized: bool,
}

struct ActiveStyleTransition {
    timeline: Timeline,
    from: ComputedStyle,
    sampled: ComputedStyle,
}

impl ActiveStyleTransition {
    fn new(
        transition: Transition,
        from_style: &ComputedStyle,
        target_style: &ComputedStyle,
    ) -> Option<Self> {
        if !has_animatable_difference(from_style, target_style) {
            return None;
        }
        Some(Self {
            timeline: Timeline::new(transition),
            from: from_style.clone(),
            sampled: interpolate_style(from_style, target_style, 0.0),
        })
    }

    fn sync_target(&mut self, target: &ComputedStyle) {
        // Preserve the animated sample while immediately accepting target
        // values for fields that have no continuous representation.
        self.sampled = interpolate_style(&self.sampled, target, 0.0);
    }

    fn tick(&mut self, delta: Duration, target: &ComputedStyle) -> bool {
        let progress = self.timeline.tick(delta);
        if progress.completed {
            self.sampled = target.clone();
            return true;
        }
        self.sampled = interpolate_style(&self.from, target, progress.eased);
        false
    }
}

/// Dense computed-style cache plus sparse animation overrides. Topology always
/// comes from `HostTree`; this system never stores parent or child links.
pub(crate) struct StyleSystem {
    nodes: SecondaryMap<NodeId, StyleNode>,
    animations: SparseSecondaryMap<NodeId, ActiveStyleTransition>,
    inherited_samples: SparseSecondaryMap<NodeId, ComputedStyle>,
    style_dirty_list: Vec<NodeId>,
    subtree_dirty_list: Vec<NodeId>,
    default_style: ComputedStyle,
}

impl StyleSystem {
    pub(crate) fn new(default_style: ComputedStyle) -> Self {
        Self {
            nodes: SecondaryMap::new(),
            animations: SparseSecondaryMap::new(),
            inherited_samples: SparseSecondaryMap::new(),
            style_dirty_list: Vec::new(),
            subtree_dirty_list: Vec::new(),
            default_style,
        }
    }

    pub(crate) fn create(&mut self, id: NodeId, computed: ComputedStyle, initialized: bool) {
        self.nodes.insert(
            id,
            StyleNode {
                computed,
                initialized,
            },
        );
    }

    pub(crate) fn remove(&mut self, id: NodeId) {
        self.nodes.remove(id);
        self.animations.remove(id);
        self.inherited_samples.remove(id);
    }

    pub(crate) fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(id)
    }

    pub(crate) fn default_style(&self) -> &ComputedStyle {
        &self.default_style
    }

    pub(crate) fn set_default_style(&mut self, style: ComputedStyle) {
        self.default_style = style;
    }

    pub(crate) fn computed(&self, id: NodeId) -> Option<&ComputedStyle> {
        self.nodes.get(id).map(|node| &node.computed)
    }

    pub(crate) fn effective(&self, id: NodeId) -> Option<&ComputedStyle> {
        let node = self.nodes.get(id)?;
        Some(
            self.animations
                .get(id)
                .map(|animation| &animation.sampled)
                .or_else(|| self.inherited_samples.get(id))
                .unwrap_or(&node.computed),
        )
    }

    pub(crate) fn styles(&self, id: NodeId) -> Option<(&ComputedStyle, &ComputedStyle)> {
        let target = self.computed(id)?;
        let effective = self
            .animations
            .get(id)
            .map(|animation| &animation.sampled)
            .or_else(|| self.inherited_samples.get(id))
            .unwrap_or(target);
        Some((target, effective))
    }

    pub(crate) fn set_computed(&mut self, id: NodeId, computed: ComputedStyle) {
        self.nodes.get_mut(id).expect("style node missing").computed = computed;
        self.inherited_samples.remove(id);
    }

    /// Re-resolves inherited text values against the parent's sampled style
    /// while leaving this node's target computed style untouched.
    pub(crate) fn sync_inherited_text(
        &mut self,
        id: NodeId,
        parent: &ComputedStyle,
        patch: &StylePatch,
    ) -> (StyleDiffFlags, bool) {
        let before = self.effective(id).expect("style node missing").clone();
        if let Some(animation) = self.animations.get_mut(id) {
            apply_sampled_text_inheritance(&mut animation.sampled, parent, patch);
            self.inherited_samples.remove(id);
        } else {
            let computed = self
                .nodes
                .get(id)
                .expect("style node missing")
                .computed
                .clone();
            let mut sampled = computed.clone();
            apply_sampled_text_inheritance(&mut sampled, parent, patch);
            if sampled == computed {
                self.inherited_samples.remove(id);
            } else {
                self.inherited_samples.insert(id, sampled);
            }
        }
        let after = self.effective(id).expect("style node missing");
        (
            before.diff(after),
            animated_sample_requires_layout(&before, after),
        )
    }

    pub(crate) fn initialized(&self, id: NodeId) -> bool {
        self.nodes.get(id).is_some_and(|node| node.initialized)
    }

    pub(crate) fn set_initialized(&mut self, id: NodeId) {
        self.nodes
            .get_mut(id)
            .expect("style node missing")
            .initialized = true;
    }

    pub(crate) fn start_transition(
        &mut self,
        id: NodeId,
        transition: Transition,
        from_style: &ComputedStyle,
        target_style: &ComputedStyle,
    ) -> bool {
        let Some(animation) = ActiveStyleTransition::new(transition, from_style, target_style)
        else {
            self.animations.remove(id);
            return false;
        };
        self.animations.insert(id, animation);
        true
    }

    pub(crate) fn remove_transition(&mut self, id: NodeId) -> bool {
        self.animations.remove(id).is_some()
    }

    pub(crate) fn sync_transition_target(&mut self, id: NodeId, target: &ComputedStyle) {
        if let Some(animation) = self.animations.get_mut(id) {
            animation.sync_target(target);
        }
    }

    pub(crate) fn tick(
        &mut self,
        delta: Duration,
        _theme: &Theme,
    ) -> Vec<(NodeId, StyleDiffFlags, bool)> {
        let active = std::mem::take(&mut self.animations);
        let mut remaining = SparseSecondaryMap::new();
        let mut changed = Vec::with_capacity(active.len());
        for (id, mut animation) in active {
            let Some(target) = self.nodes.get(id).map(|node| &node.computed) else {
                continue;
            };
            let before = animation.sampled.clone();
            let completed = animation.tick(delta, target);
            let diff = before.diff(&animation.sampled);
            if !diff.is_empty() {
                changed.push((
                    id,
                    diff,
                    animated_sample_requires_layout(&before, &animation.sampled),
                ));
            }
            if !completed {
                remaining.insert(id, animation);
            }
        }
        self.animations = remaining;
        changed
    }

    pub(crate) fn is_animating(&self) -> bool {
        !self.animations.is_empty()
    }

    pub(crate) fn mark_dirty(&mut self, id: NodeId) {
        self.style_dirty_list.push(id);
    }

    pub(crate) fn mark_subtree_dirty(&mut self, id: NodeId) {
        self.subtree_dirty_list.push(id);
    }

    pub(crate) fn drain_dirty(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.style_dirty_list)
    }

    pub(crate) fn drain_subtree_dirty(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.subtree_dirty_list)
    }

    pub(crate) fn has_dirty(&self) -> bool {
        !self.style_dirty_list.is_empty() || !self.subtree_dirty_list.is_empty()
    }
}

macro_rules! any_field_changed {
    ($from:expr, $to:expr; $($field:ident),+ $(,)?) => {
        false $(|| $from.$field != $to.$field)+
    };
}

fn animated_sample_requires_layout(from: &ComputedStyle, to: &ComputedStyle) -> bool {
    from.layout != to.layout
        || any_field_changed!(
            from.text,
            to.text;
            font_family,
            font_size,
            font_weight,
            font_style,
            line_height,
            letter_spacing,
        )
        || from.scroll.scrollbar.width != to.scroll.scrollbar.width
}

fn apply_sampled_text_inheritance(
    sampled: &mut ComputedStyle,
    parent: &ComputedStyle,
    patch: &StylePatch,
) {
    macro_rules! inherit_text_fields {
        ($($field:ident),+ $(,)?) => {
            $(
                if matches!(
                    &patch.text.$field,
                    StyleValue::Unset | StyleValue::Inherit
                ) {
                    sampled.text.$field = parent.text.$field.clone();
                }
            )+
        };
    }

    inherit_text_fields!(
        color,
        font_family,
        font_size,
        font_weight,
        font_style,
        line_height,
        letter_spacing,
        decoration,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::SlotMap;
    use xui_interface::{Color, ComputedColorStyle, StylePatch};

    #[test]
    fn animation_override_is_sparse_and_falls_back_to_computed_style() {
        let theme = Theme::default();
        let initial = ComputedStyle::initial(&theme);
        let target = ComputedStyle::compute(
            &initial,
            &StylePatch::new().background(Color::BLACK),
            &theme,
        );
        let mut ids = SlotMap::<NodeId, ()>::with_key();
        let id = ids.insert(());
        let mut styles = StyleSystem::new(initial.clone());
        styles.create(id, initial.clone(), true);

        assert_eq!(styles.effective(id), Some(&initial));
        assert!(styles.start_transition(
            id,
            Transition::new(Duration::from_millis(100)),
            &initial,
            &target,
        ));
        styles.set_computed(id, target.clone());
        assert_eq!(styles.effective(id), Some(&initial));
        assert!(styles.is_animating());

        let changed = styles.tick(Duration::from_millis(100), &theme);
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].0, id);
        assert!(!styles.is_animating());
        assert_eq!(styles.effective(id), Some(&target));

        styles.remove(id);
        assert!(!styles.contains(id));
        assert!(styles.effective(id).is_none());
    }

    #[test]
    fn text_color_animates_but_clip_is_discrete() {
        let theme = Theme::default();
        let initial = ComputedStyle::initial(&theme);
        let mut text_target = initial.clone();
        text_target.text.color = Color::WHITE;
        let mut clip_target = initial.clone();
        clip_target.paint.clip = true;
        let mut ids = SlotMap::<NodeId, ()>::with_key();
        let id = ids.insert(());
        let mut styles = StyleSystem::new(initial.clone());
        styles.create(id, initial.clone(), true);
        let transition = Transition::new(Duration::from_millis(100));

        assert!(styles.start_transition(id, transition, &initial, &text_target));
        styles.remove_transition(id);
        assert!(!styles.start_transition(id, transition, &initial, &clip_target));
        assert!(!styles.is_animating());
    }

    #[test]
    fn syncing_target_keeps_animated_paint_and_applies_discrete_values() {
        let theme = Theme::default();
        let initial = ComputedStyle::initial(&theme);
        let target = ComputedStyle::compute(
            &initial,
            &StylePatch::new().background(Color::WHITE),
            &theme,
        );
        let mut ids = SlotMap::<NodeId, ()>::with_key();
        let id = ids.insert(());
        let mut styles = StyleSystem::new(initial.clone());
        styles.create(id, initial.clone(), true);
        assert!(styles.start_transition(
            id,
            Transition::new(Duration::from_millis(100)),
            &initial,
            &target,
        ));
        styles.set_computed(id, target.clone());
        styles.tick(Duration::from_millis(50), &theme);

        let mut updated_target = target;
        updated_target.paint.clip = true;
        styles.sync_transition_target(id, &updated_target);
        styles.set_computed(id, updated_target);

        let effective = styles.effective(id).unwrap();
        assert!(effective.paint.clip);
        let ComputedColorStyle::Solid(background) = effective.paint.background else {
            panic!("expected solid background")
        };
        assert!((background.r - 0.5).abs() < 0.0001);
    }

    #[test]
    fn only_metric_samples_require_layout() {
        let theme = Theme::default();
        let initial = ComputedStyle::initial(&theme);

        let mut color = initial.clone();
        color.text.color = Color::WHITE;
        assert!(!animated_sample_requires_layout(&initial, &color));

        let mut width = initial.clone();
        width.layout.width = xui_interface::Sizing::fix(80.0);
        assert!(animated_sample_requires_layout(&initial, &width));

        let mut font_size = initial.clone();
        font_size.text.font_size += 4.0;
        assert!(animated_sample_requires_layout(&initial, &font_size));

        let mut scrollbar_color = initial.clone();
        scrollbar_color.scroll.scrollbar.thumb_color = ComputedColorStyle::Solid(Color::WHITE);
        assert!(!animated_sample_requires_layout(&initial, &scrollbar_color));
    }
}
