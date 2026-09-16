use slotmap::SlotMap;
use xui_interface::{Affine, Bounds, Point};

use super::{
    CachePolicy, ClipShape, CompositeStyle, LayerCacheKey, LayerDescriptor, Primitive,
    RenderNodeId, SceneError, ScenePatch,
};

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct RenderDirty: u16 {
        const TOPOLOGY   = 1 << 0;
        const GEOMETRY   = 1 << 1;
        const PAINT      = 1 << 2;
        const CLIP       = 1 << 3;
        const COMPOSITE  = 1 << 4;
        const EFFECT     = 1 << 5;
        const VISIBILITY = 1 << 6;

        const CONTENT = Self::TOPOLOGY.bits()
            | Self::GEOMETRY.bits()
            | Self::PAINT.bits()
            | Self::CLIP.bits()
            | Self::EFFECT.bits()
            | Self::VISIBILITY.bits();
        const ALL = Self::CONTENT.bits() | Self::COMPOSITE.bits();
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderEpochs {
    pub topology: u64,
    pub geometry: u64,
    pub paint: u64,
    pub composite: u64,
}

impl RenderEpochs {
    fn mark(&mut self, dirty: RenderDirty, revision: u64) {
        if dirty.contains(RenderDirty::TOPOLOGY) {
            self.topology = revision;
        }
        if dirty.intersects(RenderDirty::GEOMETRY | RenderDirty::CLIP | RenderDirty::VISIBILITY) {
            self.geometry = revision;
        }
        if dirty.intersects(RenderDirty::PAINT | RenderDirty::EFFECT) {
            self.paint = revision;
        }
        if dirty.contains(RenderDirty::COMPOSITE) {
            self.composite = revision;
        }
    }

    pub fn content_version(self) -> ContentVersion {
        ContentVersion {
            topology: self.topology,
            geometry: self.geometry,
            paint: self.paint,
            dynamic: 0,
        }
    }

    fn newest(self) -> u64 {
        self.topology
            .max(self.geometry)
            .max(self.paint)
            .max(self.composite)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ContentVersion {
    pub topology: u64,
    pub geometry: u64,
    pub paint: u64,
    pub dynamic: u64,
}

impl ContentVersion {
    pub fn merge(self, other: Self) -> Self {
        Self {
            topology: self.topology.max(other.topology),
            geometry: self.geometry.max(other.geometry),
            paint: self.paint.max(other.paint),
            dynamic: self.dynamic.max(other.dynamic),
        }
    }
}

#[derive(Debug)]
pub struct RenderScene {
    nodes: SlotMap<RenderNodeId, RenderNode>,
    root: RenderNodeId,
    revision: u64,
    dirty_nodes: Vec<RenderNodeId>,
}

impl Default for RenderScene {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderScene {
    pub fn new() -> Self {
        let mut nodes = SlotMap::with_key();
        let root = nodes.insert(RenderNode::group());
        let mut scene = Self {
            nodes,
            root,
            revision: 0,
            dirty_nodes: Vec::new(),
        };
        scene.mark_dirty(root, RenderDirty::ALL);
        scene
    }

    pub fn root(&self) -> RenderNodeId {
        self.root
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn node(&self, id: RenderNodeId) -> Option<&RenderNode> {
        self.nodes.get(id)
    }

    pub(crate) fn node_mut(&mut self, id: RenderNodeId) -> Option<&mut RenderNode> {
        self.nodes.get_mut(id)
    }

    pub fn children(&self, id: RenderNodeId) -> Result<&[RenderNodeId], SceneError> {
        Ok(self
            .nodes
            .get(id)
            .ok_or(SceneError::MissingNode(id))?
            .children())
    }

    pub fn contains(&self, id: RenderNodeId) -> bool {
        self.nodes.contains_key(id)
    }

    pub fn is_dirty(&self) -> bool {
        !self.dirty_nodes.is_empty()
    }

    pub fn insert_node(&mut self, mut node: RenderNode) -> RenderNodeId {
        node.parent = None;
        node.dirty = RenderDirty::empty();
        node.subtree_dirty = RenderDirty::empty();
        node.dirty_enqueued = false;
        let id = self.nodes.insert(node);
        self.mark_dirty(id, RenderDirty::ALL);
        id
    }

    pub fn insert_group(&mut self) -> RenderNodeId {
        self.insert_node(RenderNode::group())
    }

    pub fn insert_primitive(&mut self, primitive: Primitive) -> RenderNodeId {
        self.insert_node(RenderNode::primitive(primitive))
    }

    pub fn insert_transform(&mut self, transform: Affine) -> RenderNodeId {
        self.insert_node(RenderNode::transform(transform))
    }

    pub fn insert_clip(&mut self, clip: ClipShape) -> RenderNodeId {
        self.insert_node(RenderNode::clip(clip))
    }

    pub fn insert_layer(&mut self, descriptor: LayerDescriptor) -> RenderNodeId {
        self.insert_node(RenderNode::layer(descriptor))
    }

    pub fn append_child(
        &mut self,
        parent: RenderNodeId,
        child: RenderNodeId,
    ) -> Result<(), SceneError> {
        match &self
            .nodes
            .get(parent)
            .ok_or(SceneError::MissingNode(parent))?
            .kind
        {
            RenderNodeKind::Group(_) => {}
            RenderNodeKind::Primitive(_) => {
                return Err(SceneError::NodeCannotHaveChildren(parent));
            }
            _ => return Err(SceneError::UseSingleChildApi(parent)),
        }
        #[cfg(debug_assertions)]
        self.validate_attach(parent, child)?;

        match &mut self
            .nodes
            .get_mut(parent)
            .ok_or(SceneError::MissingNode(parent))?
            .kind
        {
            RenderNodeKind::Group(group) => group.children.push(child),
            RenderNodeKind::Primitive(_) => {
                return Err(SceneError::NodeCannotHaveChildren(parent));
            }
            _ => return Err(SceneError::UseSingleChildApi(parent)),
        }

        self.nodes[child].parent = Some(parent);
        self.mark_dirty(parent, RenderDirty::TOPOLOGY);
        Ok(())
    }

    pub fn insert_child(
        &mut self,
        parent: RenderNodeId,
        index: usize,
        child: RenderNodeId,
    ) -> Result<(), SceneError> {
        match &self
            .nodes
            .get(parent)
            .ok_or(SceneError::MissingNode(parent))?
            .kind
        {
            RenderNodeKind::Group(_) => {}
            RenderNodeKind::Primitive(_) => {
                return Err(SceneError::NodeCannotHaveChildren(parent));
            }
            _ => return Err(SceneError::UseSingleChildApi(parent)),
        }
        #[cfg(debug_assertions)]
        self.validate_attach(parent, child)?;

        let children = match &mut self
            .nodes
            .get_mut(parent)
            .ok_or(SceneError::MissingNode(parent))?
            .kind
        {
            RenderNodeKind::Group(group) => &mut group.children,
            RenderNodeKind::Primitive(_) => {
                return Err(SceneError::NodeCannotHaveChildren(parent));
            }
            _ => return Err(SceneError::UseSingleChildApi(parent)),
        };

        let len = children.len();
        if index > len {
            return Err(SceneError::InvalidChildIndex { parent, index, len });
        }

        children.insert(index, child);
        self.nodes[child].parent = Some(parent);
        self.mark_dirty(parent, RenderDirty::TOPOLOGY);
        Ok(())
    }

    pub fn set_child(
        &mut self,
        parent: RenderNodeId,
        child: Option<RenderNodeId>,
    ) -> Result<(), SceneError> {
        let old = match &self
            .nodes
            .get(parent)
            .ok_or(SceneError::MissingNode(parent))?
            .kind
        {
            RenderNodeKind::Transform(node) => node.child,
            RenderNodeKind::Clip(node) => node.child,
            RenderNodeKind::Layer(node) => node.child,
            RenderNodeKind::Primitive(_) => {
                return Err(SceneError::NodeCannotHaveChildren(parent));
            }
            RenderNodeKind::Group(_) => {
                return Err(SceneError::UseGroupChildrenApi(parent));
            }
        };

        if old == child {
            return Ok(());
        }

        if let Some(child) = child {
            self.validate_attach(parent, child)?;
        }

        if let Some(old) = old {
            self.nodes[old].parent = None;
        }

        match &mut self.nodes[parent].kind {
            RenderNodeKind::Transform(node) => node.child = child,
            RenderNodeKind::Clip(node) => node.child = child,
            RenderNodeKind::Layer(node) => node.child = child,
            _ => unreachable!("parent kind was validated above"),
        }

        if let Some(child) = child {
            self.nodes[child].parent = Some(parent);
        }

        self.mark_dirty(parent, RenderDirty::TOPOLOGY);
        Ok(())
    }

    pub fn replace_child(
        &mut self,
        parent: RenderNodeId,
        old: RenderNodeId,
        new: RenderNodeId,
    ) -> Result<(), SceneError> {
        if old == new {
            if self.nodes.get(old).and_then(|node| node.parent) == Some(parent) {
                return Ok(());
            }
            return Err(SceneError::ChildNotFound { parent, child: old });
        }
        #[cfg(debug_assertions)]
        self.validate_attach(parent, new)?;
        match &self.nodes[parent].kind {
            RenderNodeKind::Group(group) => {
                let index = group
                    .children
                    .iter()
                    .position(|id| *id == old)
                    .ok_or(SceneError::ChildNotFound { parent, child: old })?;
                self.nodes[old].parent = None;
                self.group_children_mut(parent)?[index] = new;
            }
            RenderNodeKind::Transform(node) if node.child == Some(old) => {
                self.nodes[old].parent = None;
                self.set_single_child_value(parent, Some(new))?;
            }
            RenderNodeKind::Clip(node) if node.child == Some(old) => {
                self.nodes[old].parent = None;
                self.set_single_child_value(parent, Some(new))?;
            }
            RenderNodeKind::Layer(node) if node.child == Some(old) => {
                self.nodes[old].parent = None;
                self.set_single_child_value(parent, Some(new))?;
            }
            RenderNodeKind::Primitive(_) => {
                return Err(SceneError::NodeCannotHaveChildren(parent));
            }
            _ => return Err(SceneError::ChildNotFound { parent, child: old }),
        }
        self.nodes[new].parent = Some(parent);
        self.mark_dirty(parent, RenderDirty::TOPOLOGY);
        Ok(())
    }

    pub fn remove_child(
        &mut self,
        parent: RenderNodeId,
        child: RenderNodeId,
    ) -> Result<(), SceneError> {
        self.remove_child_reference(parent, child)?;
        self.nodes[child].parent = None;
        self.mark_dirty(parent, RenderDirty::TOPOLOGY);
        Ok(())
    }

    pub fn reorder_child(
        &mut self,
        parent: RenderNodeId,
        child: RenderNodeId,
        new_index: usize,
    ) -> Result<bool, SceneError> {
        #[cfg(debug_assertions)]
        self.validate_group(parent)?;
        let children = self.group_children(parent)?;
        let old_index = children
            .iter()
            .position(|id| *id == child)
            .ok_or(SceneError::ChildNotFound { parent, child })?;
        if new_index >= children.len() {
            return Err(SceneError::InvalidChildIndex {
                parent,
                index: new_index,
                len: children.len(),
            });
        }
        if old_index == new_index {
            return Ok(false);
        }
        let children = self.group_children_mut(parent)?;
        let child = children.remove(old_index);
        children.insert(new_index, child);
        self.mark_dirty(parent, RenderDirty::TOPOLOGY);
        Ok(true)
    }

    pub fn detach(&mut self, child: RenderNodeId) -> Result<bool, SceneError> {
        let parent = self
            .nodes
            .get(child)
            .ok_or(SceneError::MissingNode(child))?
            .parent;
        let Some(parent) = parent else {
            return Ok(false);
        };
        self.remove_child(parent, child)?;
        Ok(true)
    }

    pub fn remove_subtree(&mut self, root: RenderNodeId) -> Result<(), SceneError> {
        if root == self.root {
            return Err(SceneError::CannotRemoveRoot);
        }
        if !self.nodes.contains_key(root) {
            return Err(SceneError::MissingNode(root));
        }
        self.detach(root)?;
        let mut stack = vec![root];
        let mut ids = Vec::new();
        while let Some(id) = stack.pop() {
            stack.extend_from_slice(self.nodes[id].children());
            ids.push(id);
        }
        for id in ids.into_iter().rev() {
            self.nodes.remove(id);
        }
        Ok(())
    }

    pub fn update_primitive(
        &mut self,
        id: RenderNodeId,
        primitive: Primitive,
    ) -> Result<bool, SceneError> {
        let current = match &self.nodes.get(id).ok_or(SceneError::MissingNode(id))?.kind {
            RenderNodeKind::Primitive(node) => &node.primitive,
            _ => return Err(self.wrong_kind(id, "Primitive")),
        };
        let change = current.diff(&primitive);
        if !change.geometry && !change.paint {
            return Ok(false);
        }
        if let RenderNodeKind::Primitive(node) = &mut self.nodes[id].kind {
            node.primitive = primitive;
        }
        let mut dirty = RenderDirty::empty();
        if change.geometry {
            dirty |= RenderDirty::GEOMETRY;
        }
        if change.paint {
            dirty |= RenderDirty::PAINT;
        }
        self.mark_dirty(id, dirty);
        Ok(true)
    }

    pub fn update_transform(
        &mut self,
        id: RenderNodeId,
        transform: Affine,
    ) -> Result<bool, SceneError> {
        let node = self.nodes.get_mut(id).ok_or(SceneError::MissingNode(id))?;
        let RenderNodeKind::Transform(value) = &mut node.kind else {
            return Err(self.wrong_kind(id, "Transform"));
        };
        if value.transform == transform {
            return Ok(false);
        }
        value.transform = transform;
        self.mark_dirty(id, RenderDirty::GEOMETRY);
        Ok(true)
    }

    pub fn update_clip(&mut self, id: RenderNodeId, clip: ClipShape) -> Result<bool, SceneError> {
        let node = self.nodes.get_mut(id).ok_or(SceneError::MissingNode(id))?;
        let RenderNodeKind::Clip(value) = &mut node.kind else {
            return Err(self.wrong_kind(id, "Clip"));
        };
        if value.clip == clip {
            return Ok(false);
        }
        value.clip = clip;
        self.mark_dirty(id, RenderDirty::CLIP | RenderDirty::GEOMETRY);
        Ok(true)
    }

    pub fn update_layer_descriptor(
        &mut self,
        id: RenderNodeId,
        descriptor: LayerDescriptor,
    ) -> Result<bool, SceneError> {
        let old = match &self.nodes.get(id).ok_or(SceneError::MissingNode(id))?.kind {
            RenderNodeKind::Layer(value) => value.descriptor.clone(),
            _ => return Err(self.wrong_kind(id, "Layer")),
        };
        if old == descriptor {
            return Ok(false);
        }
        let mut dirty = RenderDirty::empty();
        if old.bounds != descriptor.bounds {
            dirty |= RenderDirty::GEOMETRY;
        }
        if old.composite != descriptor.composite || old.backdrop_style != descriptor.backdrop_style
        {
            dirty |= RenderDirty::COMPOSITE;
        }
        if old.effects != descriptor.effects
            || old.force_offscreen != descriptor.force_offscreen
            || old.cache_policy != descriptor.cache_policy
            || old.cache_key != descriptor.cache_key
            || old.backdrop_isolation != descriptor.backdrop_isolation
        {
            dirty |= RenderDirty::EFFECT;
        }
        if let RenderNodeKind::Layer(value) = &mut self.nodes[id].kind {
            value.descriptor = descriptor;
        }
        self.mark_dirty(id, dirty);
        Ok(true)
    }

    pub fn update_layer_composite(
        &mut self,
        id: RenderNodeId,
        composite: CompositeStyle,
    ) -> Result<bool, SceneError> {
        let node = self.nodes.get_mut(id).ok_or(SceneError::MissingNode(id))?;
        let RenderNodeKind::Layer(value) = &mut node.kind else {
            return Err(self.wrong_kind(id, "Layer"));
        };
        if value.descriptor.composite == composite {
            return Ok(false);
        }
        value.descriptor.composite = composite;
        self.mark_dirty(id, RenderDirty::COMPOSITE);
        Ok(true)
    }

    pub fn set_visible(&mut self, id: RenderNodeId, visible: bool) -> Result<bool, SceneError> {
        let node = self.nodes.get_mut(id).ok_or(SceneError::MissingNode(id))?;
        if node.visible == visible {
            return Ok(false);
        }
        node.visible = visible;
        self.mark_dirty(id, RenderDirty::VISIBILITY);
        Ok(true)
    }

    fn mark_dirty(&mut self, id: RenderNodeId, dirty: RenderDirty) {
        if dirty.is_empty() {
            return;
        }
        self.revision = self.revision.wrapping_add(1).max(1);
        let revision = self.revision;
        let node = &mut self.nodes[id];
        node.dirty |= dirty;
        node.subtree_dirty |= dirty;
        node.epochs.mark(dirty, revision);
        if !node.dirty_enqueued {
            node.dirty_enqueued = true;
            self.dirty_nodes.push(id);
        }
        let mut ancestor = node.parent;
        while let Some(parent) = ancestor {
            let node = &mut self.nodes[parent];
            let missing = dirty - node.subtree_dirty;
            if missing.is_empty() {
                break;
            }
            node.subtree_dirty |= dirty;
            ancestor = node.parent;
        }
    }

    pub fn dirty_snapshot(&self) -> DirtySnapshot {
        DirtySnapshot {
            revision: self.revision,
            nodes: self.dirty_nodes.clone(),
        }
    }

    pub fn take_dirty_nodes(&mut self) -> Vec<RenderNodeId> {
        let nodes = std::mem::take(&mut self.dirty_nodes);
        for id in &nodes {
            if let Some(node) = self.nodes.get_mut(*id) {
                node.dirty_enqueued = false;
            }
        }
        nodes
            .into_iter()
            .filter(|id| self.nodes.contains_key(*id))
            .collect()
    }

    pub fn clear_node_dirty(&mut self, id: RenderNodeId, dirty: RenderDirty) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.dirty.remove(dirty);
        }
    }

    pub fn acknowledge(&mut self, snapshot: &DirtySnapshot) {
        for id in &snapshot.nodes {
            let Some(node) = self.nodes.get_mut(*id) else {
                continue;
            };
            if node.epochs.newest() <= snapshot.revision {
                node.dirty = RenderDirty::empty();
                node.dirty_enqueued = false;
            }
        }
        self.dirty_nodes.retain(|id| {
            self.nodes
                .get(*id)
                .is_some_and(|node| node.dirty_enqueued && !node.dirty.is_empty())
        });
        // `mark_dirty` keeps `subtree_dirty` correct on the way in, so only the
        // nodes whose flags were just cleared need their ancestor summaries
        // recomputed. A full-scene rebuild here would be O(scene) every frame.
        for id in &snapshot.nodes {
            self.refresh_subtree_dirty_from(*id);
        }
    }

    /// Recomputes `subtree_dirty` from `from` up to the root, stopping as soon
    /// as a node's summary is unchanged: nothing above it can change either.
    fn refresh_subtree_dirty_from(&mut self, from: RenderNodeId) {
        let mut current = Some(from);
        while let Some(id) = current {
            let Some(node) = self.nodes.get(id) else {
                break;
            };
            let parent = node.parent;
            let mut summary = node.dirty;
            for child in node.children() {
                if let Some(child) = self.nodes.get(*child) {
                    summary |= child.subtree_dirty;
                }
            }

            let node = &mut self.nodes[id];
            if node.subtree_dirty == summary {
                break;
            }
            node.subtree_dirty = summary;
            current = parent;
        }
    }

    /// Recomputes every `subtree_dirty` summary from scratch. Only needed after
    /// bulk edits that bypass `mark_dirty`; the per-frame path is incremental.
    pub fn rebuild_subtree_dirty(&mut self) {
        fn visit(scene: &mut RenderScene, id: RenderNodeId) -> RenderDirty {
            let children = scene.nodes[id].children().to_vec();
            let mut dirty = scene.nodes[id].dirty;
            for child in children {
                dirty |= visit(scene, child);
            }
            scene.nodes[id].subtree_dirty = dirty;
            dirty
        }
        visit(self, self.root);
    }

    pub fn depth_first(&self, root: RenderNodeId) -> Result<DepthFirst<'_>, SceneError> {
        if !self.nodes.contains_key(root) {
            return Err(SceneError::MissingNode(root));
        }
        Ok(DepthFirst {
            scene: self,
            stack: vec![root],
        })
    }

    pub fn apply_patch(&mut self, patch: ScenePatch) -> Result<(), SceneError> {
        match patch {
            ScenePatch::SetVisible { node, visible } => self.set_visible(node, visible).map(drop),
            ScenePatch::UpdatePrimitive { node, primitive } => {
                self.update_primitive(node, primitive).map(drop)
            }
            ScenePatch::UpdateTransform { node, transform } => {
                self.update_transform(node, transform).map(drop)
            }
            ScenePatch::UpdateClip { node, clip } => self.update_clip(node, clip).map(drop),
            ScenePatch::UpdateLayerComposite { node, composite } => {
                self.update_layer_composite(node, composite).map(drop)
            }
            ScenePatch::UpdateLayer { node, descriptor } => {
                self.update_layer_descriptor(node, descriptor).map(drop)
            }
            ScenePatch::RemoveSubtree { node } => self.remove_subtree(node),
        }
    }

    pub fn apply_patches(
        &mut self,
        patches: impl IntoIterator<Item = ScenePatch>,
    ) -> Result<(), SceneError> {
        for patch in patches {
            self.apply_patch(patch)?;
        }
        Ok(())
    }

    fn wrong_kind(&self, node: RenderNodeId, expected: &'static str) -> SceneError {
        SceneError::WrongNodeKind { node, expected }
    }

    fn validate_group(&self, parent: RenderNodeId) -> Result<(), SceneError> {
        match &self
            .nodes
            .get(parent)
            .ok_or(SceneError::MissingNode(parent))?
            .kind
        {
            RenderNodeKind::Group(_) => Ok(()),
            RenderNodeKind::Primitive(_) => Err(SceneError::NodeCannotHaveChildren(parent)),
            _ => Err(SceneError::UseSingleChildApi(parent)),
        }
    }

    fn validate_attach(&self, parent: RenderNodeId, child: RenderNodeId) -> Result<(), SceneError> {
        if !self.nodes.contains_key(parent) {
            return Err(SceneError::MissingNode(parent));
        }
        let child_node = self
            .nodes
            .get(child)
            .ok_or(SceneError::MissingNode(child))?;
        if child_node.parent == Some(parent) {
            return Err(SceneError::DuplicateChild { parent, child });
        }
        if child_node.parent.is_some() {
            return Err(SceneError::AlreadyHasParent(child));
        }
        let mut ancestor = Some(parent);
        while let Some(id) = ancestor {
            if id == child {
                return Err(SceneError::CycleDetected { parent, child });
            }
            ancestor = self.nodes[id].parent;
        }
        Ok(())
    }

    fn group_children(&self, id: RenderNodeId) -> Result<&Vec<RenderNodeId>, SceneError> {
        match &self.nodes.get(id).ok_or(SceneError::MissingNode(id))?.kind {
            RenderNodeKind::Group(group) => Ok(&group.children),
            RenderNodeKind::Primitive(_) => Err(SceneError::NodeCannotHaveChildren(id)),
            _ => Err(SceneError::UseSingleChildApi(id)),
        }
    }

    fn group_children_mut(
        &mut self,
        id: RenderNodeId,
    ) -> Result<&mut Vec<RenderNodeId>, SceneError> {
        match &mut self
            .nodes
            .get_mut(id)
            .ok_or(SceneError::MissingNode(id))?
            .kind
        {
            RenderNodeKind::Group(group) => Ok(&mut group.children),
            RenderNodeKind::Primitive(_) => Err(SceneError::NodeCannotHaveChildren(id)),
            _ => Err(SceneError::UseSingleChildApi(id)),
        }
    }

    fn single_child(&self, id: RenderNodeId) -> Result<Option<RenderNodeId>, SceneError> {
        match &self.nodes.get(id).ok_or(SceneError::MissingNode(id))?.kind {
            RenderNodeKind::Transform(node) => Ok(node.child),
            RenderNodeKind::Clip(node) => Ok(node.child),
            RenderNodeKind::Layer(node) => Ok(node.child),
            RenderNodeKind::Primitive(_) => Err(SceneError::NodeCannotHaveChildren(id)),
            RenderNodeKind::Group(_) => Err(SceneError::UseGroupChildrenApi(id)),
        }
    }

    fn set_single_child_value(
        &mut self,
        id: RenderNodeId,
        child: Option<RenderNodeId>,
    ) -> Result<(), SceneError> {
        match &mut self
            .nodes
            .get_mut(id)
            .ok_or(SceneError::MissingNode(id))?
            .kind
        {
            RenderNodeKind::Transform(node) => node.child = child,
            RenderNodeKind::Clip(node) => node.child = child,
            RenderNodeKind::Layer(node) => node.child = child,
            RenderNodeKind::Primitive(_) => return Err(SceneError::NodeCannotHaveChildren(id)),
            RenderNodeKind::Group(_) => return Err(SceneError::UseGroupChildrenApi(id)),
        }
        Ok(())
    }

    fn remove_child_reference(
        &mut self,
        parent: RenderNodeId,
        child: RenderNodeId,
    ) -> Result<(), SceneError> {
        match &mut self
            .nodes
            .get_mut(parent)
            .ok_or(SceneError::MissingNode(parent))?
            .kind
        {
            RenderNodeKind::Group(group) => {
                let index = group
                    .children
                    .iter()
                    .position(|id| *id == child)
                    .ok_or(SceneError::ChildNotFound { parent, child })?;
                group.children.remove(index);
            }
            RenderNodeKind::Transform(node) if node.child == Some(child) => node.child = None,
            RenderNodeKind::Clip(node) if node.child == Some(child) => node.child = None,
            RenderNodeKind::Layer(node) if node.child == Some(child) => node.child = None,
            RenderNodeKind::Primitive(_) => {
                return Err(SceneError::NodeCannotHaveChildren(parent));
            }
            _ => return Err(SceneError::ChildNotFound { parent, child }),
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct DirtySnapshot {
    pub revision: u64,
    pub nodes: Vec<RenderNodeId>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RenderNodeKind {
    Group(GroupNode),
    Primitive(PrimitiveNode),
    Transform(TransformNode),
    Clip(ClipNode),
    Layer(LayerNode),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GroupNode {
    pub children: Vec<RenderNodeId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrimitiveNode {
    pub primitive: Primitive,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransformNode {
    pub transform: Affine,
    pub child: Option<RenderNodeId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClipNode {
    pub clip: ClipShape,
    pub child: Option<RenderNodeId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayerNode {
    pub descriptor: LayerDescriptor,
    pub child: Option<RenderNodeId>,
}

#[derive(Debug, Clone)]
pub struct RenderNode {
    pub parent: Option<RenderNodeId>,
    pub kind: RenderNodeKind,
    pub visible: bool,
    pub dirty: RenderDirty,
    pub subtree_dirty: RenderDirty,
    pub epochs: RenderEpochs,
    dirty_enqueued: bool,
}

impl RenderNode {
    pub fn new(kind: RenderNodeKind) -> Self {
        Self {
            parent: None,
            kind,
            visible: true,
            dirty: RenderDirty::empty(),
            subtree_dirty: RenderDirty::empty(),
            epochs: RenderEpochs::default(),
            dirty_enqueued: false,
        }
    }

    pub fn group() -> Self {
        Self::new(RenderNodeKind::Group(GroupNode::default()))
    }

    pub fn primitive(primitive: Primitive) -> Self {
        Self::new(RenderNodeKind::Primitive(PrimitiveNode { primitive }))
    }

    pub fn transform(transform: Affine) -> Self {
        Self::new(RenderNodeKind::Transform(TransformNode {
            transform,
            child: None,
        }))
    }

    pub fn clip(clip: ClipShape) -> Self {
        Self::new(RenderNodeKind::Clip(ClipNode { clip, child: None }))
    }

    pub fn layer(descriptor: LayerDescriptor) -> Self {
        Self::new(RenderNodeKind::Layer(LayerNode {
            descriptor,
            child: None,
        }))
    }

    pub fn children(&self) -> &[RenderNodeId] {
        match &self.kind {
            RenderNodeKind::Group(group) => &group.children,
            RenderNodeKind::Transform(node) => node.child.as_slice(),
            RenderNodeKind::Clip(node) => node.child.as_slice(),
            RenderNodeKind::Layer(node) => node.child.as_slice(),
            RenderNodeKind::Primitive(_) => &[],
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct HostRenderBinding {
    /// Layout-position transform and root of this host's retained subtree.
    pub root: RenderNodeId,
    /// Visual Style transform. This remains separate from layout so frame
    /// properties can animate it without recompiling the retained scene.
    pub transform: RenderNodeId,
    pub contents: RenderNodeId,
    pub paint: RenderNodeId,
    pub children: Option<RenderNodeId>,
    pub overlay: Option<RenderNodeId>,
    pub scroll_transform: Option<RenderNodeId>,
    pub clip: Option<RenderNodeId>,
    pub layer: Option<RenderNodeId>,
    pub host_layout_epoch: u64,
    pub host_paint_epoch: u64,
    pub host_composite_epoch: u64,
}

impl HostRenderBinding {
    pub fn scaffold(
        root: RenderNodeId,
        transform: RenderNodeId,
        contents: RenderNodeId,
        paint: RenderNodeId,
        children: Option<RenderNodeId>,
        overlay: Option<RenderNodeId>,
        scroll_transform: Option<RenderNodeId>,
    ) -> Self {
        Self {
            root,
            transform,
            contents,
            paint,
            children,
            overlay,
            scroll_transform,
            clip: None,
            layer: None,
            host_layout_epoch: 0,
            host_paint_epoch: 0,
            host_composite_epoch: 0,
        }
    }

    pub(crate) fn references(&self) -> Vec<(&'static str, RenderNodeId)> {
        let mut values = vec![
            ("root", self.root),
            ("transform", self.transform),
            ("contents", self.contents),
            ("paint", self.paint),
            // ("children", self.children),
            // ("overlay", self.overlay),
            // ("scroll_transform", self.scroll_transform),
        ];

        if let Some(id) = self.children {
            values.push(("children", id));
        }
        if let Some(id) = self.overlay {
            values.push(("overlay", id));
        }
        if let Some(id) = self.scroll_transform {
            values.push(("scroll_transform", id));
        }

        if let Some(id) = self.clip {
            values.push(("clip", id));
        }
        if let Some(id) = self.layer {
            values.push(("layer", id));
        }
        values
    }
}

pub struct DepthFirst<'a> {
    scene: &'a RenderScene,
    stack: Vec<RenderNodeId>,
}

impl<'a> Iterator for DepthFirst<'a> {
    type Item = (RenderNodeId, &'a RenderNode);

    fn next(&mut self) -> Option<Self::Item> {
        let id = self.stack.pop()?;
        let node = self.scene.nodes.get(id)?;
        self.stack.extend(node.children().iter().rev().copied());
        Some((id, node))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ScrollRenderNodes {
    pub root: RenderNodeId,
    pub clip: RenderNodeId,
    pub transform: RenderNodeId,
    pub content: RenderNodeId,
}

pub fn create_scroll_scene(
    scene: &mut RenderScene,
    viewport: Bounds,
) -> Result<ScrollRenderNodes, SceneError> {
    let clip = scene.insert_clip(ClipShape::Rect(viewport));
    let transform = scene.insert_transform(Affine::IDENTITY);
    let content = scene.insert_group();
    scene.set_child(clip, Some(transform))?;
    scene.set_child(transform, Some(content))?;
    Ok(ScrollRenderNodes {
        root: clip,
        clip,
        transform,
        content,
    })
}

pub fn update_scroll_offset(
    scene: &mut RenderScene,
    nodes: &ScrollRenderNodes,
    offset: Point,
) -> Result<bool, SceneError> {
    scene.update_transform(nodes.transform, Affine::translate(-offset.x, -offset.y))
}

#[derive(Debug, Clone, Copy)]
pub struct PageRenderNodes {
    pub layer: RenderNodeId,
    pub content: RenderNodeId,
}

pub fn create_page_layer(
    scene: &mut RenderScene,
    cache_key: LayerCacheKey,
    bounds: Bounds,
) -> Result<PageRenderNodes, SceneError> {
    let descriptor = LayerDescriptor {
        bounds: Some(bounds),
        cache_key: Some(cache_key),
        cache_policy: CachePolicy::Always,
        force_offscreen: true,
        ..LayerDescriptor::default()
    };
    let layer = scene.insert_layer(descriptor);
    let content = scene.insert_group();
    scene.set_child(layer, Some(content))?;
    Ok(PageRenderNodes { layer, content })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{BlendMode, Shape, ShapePrimitive};
    use slotmap::SlotMap;
    use xui_interface::{Color, ComputedColorStyle};

    fn shape(bounds: Bounds, color: Color) -> Primitive {
        Primitive::Shape(ShapePrimitive {
            bounds,
            shape: Shape::Rect,
            fill: Some(ComputedColorStyle::Solid(color)),
            stroke: None,
            shadow: None,
        })
    }

    fn acknowledge_all(scene: &mut RenderScene) {
        let snapshot = scene.dirty_snapshot();
        scene.acknowledge(&snapshot);
    }

    #[test]
    fn root_is_permanent_group_and_stale_ids_do_not_alias() {
        let mut scene = RenderScene::new();
        assert!(matches!(
            scene.node(scene.root()).unwrap().kind,
            RenderNodeKind::Group(_)
        ));
        assert_eq!(
            scene.remove_subtree(scene.root()),
            Err(SceneError::CannotRemoveRoot)
        );

        let old = scene.insert_group();
        scene.remove_subtree(old).unwrap();
        let replacement = scene.insert_group();
        assert_ne!(old, replacement);
        assert!(!scene.contains(old));
    }

    #[test]
    fn group_order_single_child_rules_and_detach_are_enforced() {
        let mut scene = RenderScene::new();
        let group = scene.insert_group();
        let a = scene.insert_group();
        let b = scene.insert_group();
        scene.append_child(group, a).unwrap();
        scene.insert_child(group, 0, b).unwrap();
        assert_eq!(scene.children(group).unwrap(), &[b, a]);
        assert!(matches!(
            scene.append_child(group, a),
            Err(SceneError::DuplicateChild { .. })
        ));

        let transform = scene.insert_transform(Affine::IDENTITY);
        scene.set_child(transform, Some(a)).unwrap_err();
        scene.detach(a).unwrap();
        scene.set_child(transform, Some(a)).unwrap();
        assert!(matches!(
            scene.append_child(transform, b),
            Err(SceneError::UseSingleChildApi(_))
        ));
        let primitive = scene.insert_primitive(shape(Bounds::ZERO, Color::BLACK));
        assert!(matches!(
            scene.append_child(primitive, b),
            Err(SceneError::NodeCannotHaveChildren(_))
        ));
        scene.detach(a).unwrap();
        assert_eq!(scene.node(a).unwrap().parent, None);
        assert!(scene.children(transform).unwrap().is_empty());
    }

    #[test]
    fn cycles_double_parent_replace_and_reorder_are_checked() {
        let mut scene = RenderScene::new();
        let a = scene.insert_group();
        let b = scene.insert_group();
        let c = scene.insert_group();
        scene.append_child(a, b).unwrap();
        assert!(matches!(
            scene.append_child(b, a),
            Err(SceneError::CycleDetected { .. })
        ));
        assert!(matches!(
            scene.append_child(scene.root(), b),
            Err(SceneError::AlreadyHasParent(_))
        ));
        scene.append_child(a, c).unwrap();
        scene.reorder_child(a, c, 0).unwrap();
        assert_eq!(scene.children(a).unwrap(), &[c, b]);
        let replacement = scene.insert_group();
        scene.replace_child(a, c, replacement).unwrap();
        assert_eq!(scene.children(a).unwrap(), &[replacement, b]);
        assert_eq!(scene.node(c).unwrap().parent, None);
    }

    #[test]
    fn remove_subtree_removes_descendants() {
        let mut scene = RenderScene::new();
        let root = scene.insert_group();
        let child = scene.insert_group();
        let grandchild = scene.insert_group();
        scene.append_child(root, child).unwrap();
        scene.append_child(child, grandchild).unwrap();

        scene.remove_subtree(child).unwrap();
        assert!(!scene.contains(child));
        assert!(!scene.contains(grandchild));
    }

    #[test]
    fn primitive_diff_dirty_propagation_and_acknowledge_are_precise() {
        let mut scene = RenderScene::new();
        let group = scene.insert_group();
        let primitive = scene.insert_primitive(shape(
            Bounds::from_origin_size((0.0, 0.0), (10.0, 10.0)),
            Color::BLACK,
        ));
        scene.append_child(scene.root(), group).unwrap();
        scene.append_child(group, primitive).unwrap();
        acknowledge_all(&mut scene);
        let revision = scene.revision();
        assert!(
            !scene
                .update_primitive(
                    primitive,
                    shape(
                        Bounds::from_origin_size((0.0, 0.0), (10.0, 10.0)),
                        Color::BLACK
                    )
                )
                .unwrap()
        );
        assert_eq!(scene.revision(), revision);

        scene
            .update_primitive(
                primitive,
                shape(
                    Bounds::from_origin_size((1.0, 0.0), (10.0, 10.0)),
                    Color::BLACK,
                ),
            )
            .unwrap();
        assert_eq!(scene.node(primitive).unwrap().dirty, RenderDirty::GEOMETRY);
        assert!(
            scene
                .node(group)
                .unwrap()
                .subtree_dirty
                .contains(RenderDirty::GEOMETRY)
        );
        assert_eq!(scene.dirty_snapshot().nodes, vec![primitive]);
        acknowledge_all(&mut scene);
        assert!(scene.node(scene.root()).unwrap().subtree_dirty.is_empty());

        scene
            .update_primitive(
                primitive,
                shape(
                    Bounds::from_origin_size((1.0, 0.0), (10.0, 10.0)),
                    Color::WHITE,
                ),
            )
            .unwrap();
        assert_eq!(scene.node(primitive).unwrap().dirty, RenderDirty::PAINT);
    }

    #[test]
    fn transform_clip_visibility_and_composite_use_separate_epochs() {
        let mut scene = RenderScene::new();
        let transform = scene.insert_transform(Affine::IDENTITY);
        let clip = scene.insert_clip(ClipShape::Rect(Bounds::ZERO));
        let layer = scene.insert_layer(LayerDescriptor::default());
        acknowledge_all(&mut scene);

        scene
            .update_transform(transform, Affine::translate(1.0, 2.0))
            .unwrap();
        assert_eq!(scene.node(transform).unwrap().dirty, RenderDirty::GEOMETRY);
        scene
            .update_clip(
                clip,
                ClipShape::Rect(Bounds::from_origin_size((0.0, 0.0), (5.0, 5.0))),
            )
            .unwrap();
        assert_eq!(
            scene.node(clip).unwrap().dirty,
            RenderDirty::CLIP | RenderDirty::GEOMETRY
        );
        scene.set_visible(transform, false).unwrap();
        assert!(
            scene
                .node(transform)
                .unwrap()
                .dirty
                .contains(RenderDirty::VISIBILITY)
        );

        let before = scene.node(layer).unwrap().epochs.content_version();
        scene
            .update_layer_composite(
                layer,
                CompositeStyle {
                    opacity: 0.5,
                    transform: Affine::translate(3.0, 0.0),
                    blend_mode: BlendMode::Normal,
                    operator: crate::render::CompositeOperator::SrcOver,
                },
            )
            .unwrap();
        let node = scene.node(layer).unwrap();
        assert_eq!(node.dirty, RenderDirty::COMPOSITE);
        assert_eq!(node.epochs.content_version(), before);
        assert!(node.epochs.composite > 0);
    }

    #[test]
    fn scroll_and_page_helpers_preserve_content_versions() {
        let mut scene = RenderScene::new();
        let scroll = create_scroll_scene(
            &mut scene,
            Bounds::from_origin_size((0.0, 0.0), (100.0, 80.0)),
        )
        .unwrap();
        acknowledge_all(&mut scene);
        update_scroll_offset(&mut scene, &scroll, Point::new(12.0, 7.0)).unwrap();
        assert_eq!(
            scene.node(scroll.transform).unwrap().dirty,
            RenderDirty::GEOMETRY
        );

        let mut keys = SlotMap::<LayerCacheKey, ()>::with_key();
        let page = create_page_layer(
            &mut scene,
            keys.insert(()),
            Bounds::from_origin_size((0.0, 0.0), (320.0, 240.0)),
        )
        .unwrap();
        acknowledge_all(&mut scene);
        let content_before = scene.node(page.layer).unwrap().epochs.content_version();
        scene
            .update_layer_composite(
                page.layer,
                CompositeStyle {
                    opacity: 0.8,
                    transform: Affine::translate(20.0, 0.0),
                    blend_mode: BlendMode::Normal,
                    operator: crate::render::CompositeOperator::SrcOver,
                },
            )
            .unwrap();
        assert_eq!(
            scene.node(page.layer).unwrap().epochs.content_version(),
            content_before
        );
    }

    #[test]
    fn depth_first_preserves_child_order() {
        let mut scene = RenderScene::new();
        let a = scene.insert_group();
        let b = scene.insert_group();
        let c = scene.insert_group();
        scene.append_child(scene.root(), a).unwrap();
        scene.append_child(scene.root(), b).unwrap();
        scene.append_child(a, c).unwrap();
        let order: Vec<_> = scene
            .depth_first(scene.root())
            .unwrap()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(order, vec![scene.root(), a, c, b]);
    }
}
