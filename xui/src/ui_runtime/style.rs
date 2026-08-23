use crate::animation::AnimableStyle;
use slotmap::{SecondaryMap, SparseSecondaryMap};
use std::time::Duration;
use xui_animation::{Animatable, Timeline, Transition};
use xui_interface::{ComputedStyle, NodeId, Theme};

pub(crate) struct StyleNode {
    computed: ComputedStyle,
    initialized: bool,
}

struct ActiveStyleTransition {
    timeline: Timeline,
    from: AnimableStyle,
    to: AnimableStyle,
    sampled: ComputedStyle,
}

impl ActiveStyleTransition {
    fn new(
        transition: Transition,
        from_style: &ComputedStyle,
        target_style: &ComputedStyle,
    ) -> Option<Self> {
        let (from, to) = AnimableStyle::diff(from_style, target_style);
        if !to.has_properties() {
            return None;
        }
        let mut sampled = target_style.clone();
        from.apply_to_computed(&mut sampled);
        Some(Self {
            timeline: Timeline::new(transition),
            from,
            to,
            sampled,
        })
    }

    fn sync_target(&mut self, target: &ComputedStyle) {
        let current = AnimableStyle::capture(&self.sampled, &self.to);
        self.sampled = target.clone();
        current.apply_to_computed(&mut self.sampled);
    }

    fn tick(&mut self, delta: Duration, target: &ComputedStyle) -> bool {
        let progress = self.timeline.tick(delta);
        self.sampled = target.clone();
        if progress.completed {
            return true;
        }
        let interpolated = AnimableStyle::interpolate(&self.from, &self.to, progress.eased);
        interpolated.apply_to_computed(&mut self.sampled);
        false
    }
}

/// Dense computed-style cache plus sparse animation overrides. Topology always
/// comes from `HostTree`; this system never stores parent or child links.
pub(crate) struct StyleSystem {
    nodes: SecondaryMap<NodeId, StyleNode>,
    animations: SparseSecondaryMap<NodeId, ActiveStyleTransition>,
    style_dirty_list: Vec<NodeId>,
    subtree_dirty_list: Vec<NodeId>,
    default_style: ComputedStyle,
}

impl StyleSystem {
    pub(crate) fn new(default_style: ComputedStyle) -> Self {
        Self {
            nodes: SecondaryMap::new(),
            animations: SparseSecondaryMap::new(),
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
                .unwrap_or(&node.computed),
        )
    }

    pub(crate) fn styles(&self, id: NodeId) -> Option<(&ComputedStyle, &ComputedStyle)> {
        let target = self.computed(id)?;
        let effective = self
            .animations
            .get(id)
            .map(|animation| &animation.sampled)
            .unwrap_or(target);
        Some((target, effective))
    }

    pub(crate) fn set_computed(&mut self, id: NodeId, computed: ComputedStyle) {
        self.nodes.get_mut(id).expect("style node missing").computed = computed;
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

    pub(crate) fn tick(&mut self, delta: Duration, _theme: &Theme) -> Vec<NodeId> {
        let active = std::mem::take(&mut self.animations);
        let mut remaining = SparseSecondaryMap::new();
        let mut changed = Vec::with_capacity(active.len());
        for (id, mut animation) in active {
            let Some(target) = self.nodes.get(id).map(|node| &node.computed) else {
                continue;
            };
            let completed = animation.tick(delta, target);
            changed.push(id);
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

        assert_eq!(styles.tick(Duration::from_millis(100), &theme), vec![id]);
        assert!(!styles.is_animating());
        assert_eq!(styles.effective(id), Some(&target));

        styles.remove(id);
        assert!(!styles.contains(id));
        assert!(styles.effective(id).is_none());
    }

    #[test]
    fn text_and_clip_only_changes_do_not_start_paint_transition() {
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

        assert!(!styles.start_transition(id, transition, &initial, &text_target));
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
        updated_target.text.color = Color::WHITE;
        styles.sync_transition_target(id, &updated_target);
        styles.set_computed(id, updated_target);

        let effective = styles.effective(id).unwrap();
        assert!(effective.paint.clip);
        assert_eq!(effective.text.color, Color::WHITE);
        let ComputedColorStyle::Solid(background) = effective.paint.background else {
            panic!("expected solid background")
        };
        assert!((background.r - 0.5).abs() < 0.0001);
    }
}
