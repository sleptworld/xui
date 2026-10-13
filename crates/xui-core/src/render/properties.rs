use super::{FrameBuildError, RenderNodeId, RenderNodeKind, RenderScene};
use rustc_hash::FxHashMap;
use xui_interface::Affine;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FramePropertiesSnapshot {
    pub revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DynamicComposite {
    pub opacity: Option<f32>,
    pub transform: Option<Affine>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Versioned<T> {
    pub value: T,
    pub revision: u64,
}

#[derive(Debug, Clone, Default)]
pub struct FrameProperties {
    transforms: FxHashMap<RenderNodeId, Versioned<Affine>>,
    /// Fresh whenever a transform is set, changed or removed; see
    /// [`fresh_token`](super::fresh_token). 0 until the first, as every
    /// instance is empty until then.
    transform_token: u64,
    composites: FxHashMap<RenderNodeId, Versioned<DynamicComposite>>,
    revision: u64,
    acknowledged_revision: u64,
}

impl FrameProperties {
    /// Rejects a dynamic transform on anything but a transform node, and a
    /// dynamic composite on anything but an isolated layer. Properties on
    /// nodes outside the scene's tree are ignored, as the frame ignores them.
    #[cfg(debug_assertions)]
    pub fn validate(&self, scene: &RenderScene) -> Result<(), FrameBuildError> {
        let attached = |source: RenderNodeId| {
            scene
                .node(source)
                .filter(|_| scene.is_attached(source))
                .map(|node| &node.kind)
        };
        for source in self.transform_sources() {
            if attached(source).is_some_and(|kind| !matches!(kind, RenderNodeKind::Transform(_))) {
                return Err(FrameBuildError::DynamicTransformOnNonSpatialNode(source));
            }
        }
        for source in self.composite_sources() {
            let isolated = |kind: &RenderNodeKind| matches!(kind, RenderNodeKind::Layer(layer) if layer.descriptor.requires_isolation());
            if attached(source).is_some_and(|kind| !isolated(kind)) {
                return Err(FrameBuildError::DynamicCompositeOnNonIsolatedLayer(source));
            }
        }
        Ok(())
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn is_dirty(&self) -> bool {
        self.revision > self.acknowledged_revision
    }

    pub fn snapshot(&self) -> FramePropertiesSnapshot {
        FramePropertiesSnapshot {
            revision: self.revision,
        }
    }

    pub fn acknowledge(&mut self, snapshot: FramePropertiesSnapshot) {
        self.acknowledged_revision = self
            .acknowledged_revision
            .max(snapshot.revision.min(self.revision));
    }

    /// Sets the dynamic transform of the transform node `source`. It composes
    /// with the node's own transform rather than replacing it, applying first,
    /// in the node's local space: a host's style transform rides on its
    /// layout position this way, without a node of its own and without the
    /// position being folded into the value.
    pub fn set_transform(&mut self, source: RenderNodeId, transform: Affine) -> bool {
        if self
            .transforms
            .get(&source)
            .is_some_and(|current| current.value == transform)
        {
            return false;
        }
        let revision = self.next_revision();
        self.transform_token = super::fresh_token();
        self.transforms.insert(
            source,
            Versioned {
                value: transform,
                revision,
            },
        );
        true
    }

    pub fn clear_transform(&mut self, source: RenderNodeId) -> bool {
        if self.transforms.remove(&source).is_none() {
            return false;
        }
        self.next_revision();
        self.transform_token = super::fresh_token();
        true
    }

    pub fn set_composite(&mut self, source: RenderNodeId, composite: DynamicComposite) -> bool {
        if self
            .composites
            .get(&source)
            .is_some_and(|current| current.value == composite)
        {
            return false;
        }
        let revision = self.next_revision();
        self.composites.insert(
            source,
            Versioned {
                value: composite,
                revision,
            },
        );
        true
    }

    pub fn clear_composite(&mut self, source: RenderNodeId) -> bool {
        if self.composites.remove(&source).is_none() {
            return false;
        }
        self.next_revision();
        true
    }

    pub fn remove_source(&mut self, source: RenderNodeId) -> bool {
        let removed_transform = self.transforms.remove(&source).is_some();
        if removed_transform {
            self.transform_token = super::fresh_token();
        }
        let removed = removed_transform | self.composites.remove(&source).is_some();
        if removed {
            self.next_revision();
        }
        removed
    }

    pub(crate) fn transform_token(&self) -> u64 {
        self.transform_token
    }

    pub(crate) fn transform(&self, source: RenderNodeId) -> Option<Versioned<Affine>> {
        self.transforms.get(&source).copied()
    }

    pub(crate) fn composite(&self, source: RenderNodeId) -> Option<Versioned<DynamicComposite>> {
        self.composites.get(&source).copied()
    }

    pub(crate) fn composite_sources(&self) -> impl Iterator<Item = RenderNodeId> + '_ {
        self.composites.keys().copied()
    }

    pub(crate) fn transform_sources(&self) -> impl Iterator<Item = RenderNodeId> + '_ {
        self.transforms.keys().copied()
    }

    fn next_revision(&mut self) -> u64 {
        self.revision = self.revision.wrapping_add(1).max(1);
        self.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_state_is_retained_until_the_matching_snapshot_is_acknowledged() {
        let source = RenderNodeId::default();
        let mut properties = FrameProperties::default();
        properties.set_transform(source, Affine::translate(10.0, 0.0));
        let first = properties.snapshot();

        properties.set_transform(source, Affine::translate(20.0, 0.0));
        let second = properties.snapshot();
        properties.acknowledge(first);
        assert!(properties.is_dirty());

        properties.acknowledge(second);
        assert!(!properties.is_dirty());
        assert_eq!(
            properties.transform(source).unwrap().value,
            Affine::translate(20.0, 0.0)
        );
    }

    #[test]
    fn identical_updates_do_not_advance_revision() {
        let source = RenderNodeId::default();
        let mut properties = FrameProperties::default();
        assert!(properties.set_transform(source, Affine::IDENTITY));
        let revision = properties.revision();
        assert!(!properties.set_transform(source, Affine::IDENTITY));
        assert_eq!(properties.revision(), revision);
    }
}
