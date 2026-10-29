use std::{collections::HashSet, sync::Arc};

use slotmap::{SecondaryMap, SlotMap};

use super::{
    CompiledClip, CompiledClipId, CompiledItemSpan, CompiledPicture, CompiledPictureItem,
    CompiledPrimitive, CompiledScene, CompiledSpatialNode, ContentVersion, DirtySnapshot,
    LayerCacheKey, LayerDescriptor, NodeItemLayout, PictureId, RenderDirty, RenderNodeId,
    RenderNodeKind, RenderScene, SceneError, SpatialNodeId,
    compiled_scene::{CompiledEntity, CompiledNode},
    render_graph::BuiltLayerProgram,
};
use xui_interface::{Affine, core::Bounds};

#[derive(Debug, Clone, PartialEq)]
pub enum SceneCompileError {
    Scene(SceneError),
    DuplicateLayerCacheKey(LayerCacheKey),
    RenderGraph {
        source: RenderNodeId,
        error: xui_render_graph::CompileError,
    },
    RenderGraphBinding {
        source: RenderNodeId,
        error: xui_render_graph::BindingError,
    },
}

impl std::fmt::Display for SceneCompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Scene(error) => error.fmt(f),
            Self::DuplicateLayerCacheKey(key) => {
                write!(
                    f,
                    "layer cache key {key:?} is used more than once in one scene"
                )
            }
            Self::RenderGraph { source, error } => {
                write!(
                    f,
                    "failed to compile render graph for layer {source:?}: {error}"
                )
            }
            Self::RenderGraphBinding { source, error } => {
                write!(
                    f,
                    "failed to bind render graph for layer {source:?}: {error}"
                )
            }
        }
    }
}

impl std::error::Error for SceneCompileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Scene(error) => Some(error),
            Self::RenderGraph { error, .. } => Some(error),
            Self::RenderGraphBinding { error, .. } => Some(error),
            Self::DuplicateLayerCacheKey(_) => None,
        }
    }
}

impl From<SceneError> for SceneCompileError {
    fn from(value: SceneError) -> Self {
        Self::Scene(value)
    }
}

#[derive(Debug, Default)]
pub struct SceneCompiler {
    compiled: Option<CompiledScene>,
}

impl SceneCompiler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn compiled_scene(&self) -> Option<&CompiledScene> {
        self.compiled.as_ref()
    }

    pub fn compile<'a>(
        &'a mut self,
        source: &RenderScene,
        snapshot: &DirtySnapshot,
    ) -> Result<&'a CompiledScene, SceneCompileError> {
        let needs_initial = self.compiled.is_none();
        // Before anything else, and even when nothing else changed: a detached
        // subtree can be removed without dirtying the scene, and the log is
        // cleared once the frame is acknowledged.
        if let Some(compiled) = self.compiled.as_mut() {
            drop_removed_sources(compiled, source.removed_nodes());
        }
        let compiled_revision = self
            .compiled
            .as_ref()
            .map(|scene| scene.scene_revision)
            .unwrap_or(0);
        if !needs_initial && source.revision() <= compiled_revision {
            return Ok(self.compiled.as_ref().expect("compiled scene exists"));
        }

        let changed: Vec<_> = snapshot
            .nodes
            .iter()
            .copied()
            .filter_map(|id| {
                let node = source.node(id)?;
                let dirty = dirty_since(node, compiled_revision);
                (!dirty.is_empty()).then_some((id, dirty))
            })
            .collect();

        // Duplicate cache keys can only appear when a layer descriptor changes
        // (`EFFECT` covers `cache_key`) or when a subtree is attached
        // (`TOPOLOGY`). Re-walking the whole scene on every paint-only frame
        // costs O(scene) for a check that cannot have changed.
        // With fewer than two keyed layers in the whole scene there is nothing
        // to collide, which is every scene that does not page.
        #[cfg(debug_assertions)]
        if source.may_have_duplicate_cache_keys()
            && (needs_initial
                || changed.iter().any(|(_, dirty)| {
                    dirty.intersects(RenderDirty::TOPOLOGY | RenderDirty::EFFECT)
                }))
        {
            validate_explicit_cache_keys(source)?;
        }

        let structural = needs_initial
            || changed.iter().any(|(id, dirty)| {
                requires_structural_rebuild(source, self.compiled.as_ref(), *id, *dirty)
            });

        if structural {
            // Taken rather than cloned: a deep copy of every entity map costs
            // O(scene) on each structural frame.
            let mut next = match self.compiled.take() {
                Some(current) => current,
                None => empty_compiled_scene(source),
            };
            let outcome = rebuild_dirty_paths(&mut next, source);
            // A failed rebuild leaves `next` half-written, so it is dropped
            // instead of retained; `self.compiled` stays `None` and the next
            // call starts from a clean full rebuild.
            outcome?;
            next.scene_revision = source.revision();
            self.compiled = Some(next);
        } else if let Some(compiled) = self.compiled.as_mut() {
            apply_non_structural_updates(compiled, source, &changed)?;
            compiled.scene_revision = source.revision();
        }

        Ok(self
            .compiled
            .as_ref()
            .expect("scene compiler produced a scene"))
    }
}

fn dirty_since(node: &super::RenderNode, revision: u64) -> RenderDirty {
    let mut dirty = RenderDirty::empty();
    if node.epochs.topology > revision {
        dirty |= node.dirty & RenderDirty::TOPOLOGY;
    }
    if node.epochs.geometry > revision {
        dirty |= node.dirty & (RenderDirty::GEOMETRY | RenderDirty::CLIP | RenderDirty::VISIBILITY);
    }
    if node.epochs.paint > revision {
        dirty |= node.dirty & RenderDirty::PAINT;
    }
    if node.epochs.effect > revision {
        dirty |= node.dirty & RenderDirty::EFFECT;
    }
    if node.epochs.composite > revision {
        dirty |= node.dirty & RenderDirty::COMPOSITE;
    }
    dirty
}

fn requires_structural_rebuild(
    source: &RenderScene,
    compiled: Option<&CompiledScene>,
    id: RenderNodeId,
    dirty: RenderDirty,
) -> bool {
    if dirty.intersects(RenderDirty::TOPOLOGY | RenderDirty::VISIBILITY) {
        return true;
    }
    // `EFFECT` alone is not structural. Only a layer that gains or loses
    // isolation changes the picture graph, and the per-kind check below already
    // detects exactly that; an effect swap on an already-isolated layer is a
    // descriptor update that `apply_non_structural_updates` handles in place.
    let Some(node) = source.node(id) else {
        return true;
    };
    let Some(compiled) = compiled else {
        return true;
    };
    match &node.kind {
        RenderNodeKind::Group(_) => false,
        RenderNodeKind::Primitive(_) => compiled.primitive_for_source(id).is_none(),
        RenderNodeKind::Transform(_) => compiled.spatial_for_source(id).is_none(),
        RenderNodeKind::Clip(_) => compiled.clip_for_source(id).is_none(),
        RenderNodeKind::Layer(layer) => {
            layer.descriptor.requires_isolation() != compiled.picture_for_source(id).is_some()
        }
    }
}

/// Widens the subtree bounds of `spatial` and its ancestors to cover
/// `bounds`, given in `spatial`'s local space, after an in-place geometry
/// update moved something under it.
///
/// It only ever grows them, which is O(depth) where recomputing them exactly
/// would be O(scene): the outermost spans cover everything. Bounds that are
/// too large only cost culling precision, and the next walk through the node
/// records them exactly again. Clips are not re-applied either, for the same
/// reason.
fn grow_subtree_bounds(
    compiled: &mut CompiledScene,
    mut spatial: Option<SpatialNodeId>,
    mut bounds: Bounds,
) {
    while let Some(id) = spatial {
        // An entity kept for a detached subtree may sit under a spatial node
        // that has been removed since; it is placed nowhere, so nothing grows.
        let Some(node) = compiled.spatial_nodes.get_mut(id) else {
            return;
        };
        // Unbounded, or empty and so not holding what moved: either way the
        // ancestors are left as they are.
        let Some(subtree) = &mut node.subtree_bounds else {
            return;
        };
        *subtree = subtree.union(bounds);
        bounds = node.local_transform.transform_bounds(bounds);
        spatial = node.parent;
    }
}

fn apply_non_structural_updates(
    compiled: &mut CompiledScene,
    source: &RenderScene,
    changed: &[(RenderNodeId, RenderDirty)],
) -> Result<(), SceneCompileError> {
    for (id, dirty) in changed {
        let node = source.node(*id).ok_or(SceneError::MissingNode(*id))?;
        // Anything but a composite can move the bounds the node's walk
        // recorded for it and its ancestors.
        if !(*dirty - RenderDirty::COMPOSITE).is_empty() {
            invalidate_layout_bounds(compiled, source, *id);
        }
        match &node.kind {
            RenderNodeKind::Group(_) => {}
            RenderNodeKind::Primitive(value) => {
                let key = compiled
                    .primitive_for_source(*id)
                    .ok_or(SceneError::MissingNode(*id))?;
                let primitive = &mut compiled.primitives[key];
                if dirty.intersects(RenderDirty::GEOMETRY | RenderDirty::PAINT) {
                    primitive.primitive = value.primitive.clone();
                    primitive.content_version = node.epochs.content_version();
                }
            }
            RenderNodeKind::Transform(value) => {
                let key = compiled
                    .spatial_for_source(*id)
                    .ok_or(SceneError::MissingNode(*id))?;
                let spatial = &mut compiled.spatial_nodes[key];
                if dirty.contains(RenderDirty::GEOMETRY) {
                    spatial.local_transform = value.transform;
                    spatial.content_version = node.epochs.content_version();
                    compiled.spatial_token = super::fresh_token();
                }
            }
            RenderNodeKind::Clip(value) => {
                let key = compiled
                    .clip_for_source(*id)
                    .ok_or(SceneError::MissingNode(*id))?;
                let clip = &mut compiled.clips[key];
                if dirty.intersects(RenderDirty::CLIP | RenderDirty::GEOMETRY) {
                    clip.clip = value.clip.clone();
                    clip.content_version = node.epochs.content_version();
                }
            }
            RenderNodeKind::Layer(value) => {
                if let Some(picture_id) = compiled.picture_for_source(*id) {
                    let render_program =
                        reusable_render_program(compiled, picture_id, *id, &value.descriptor)?;
                    let picture = &mut compiled.pictures[picture_id];
                    picture.descriptor = value.descriptor.clone();
                    picture.render_program = Some(render_program);
                    picture.content_version = node.epochs.content_version();
                    picture.composite_version = node.epochs.composite;
                    picture.effect_version = node.epochs.effect;
                }
            }
        }
    }
    // Subtree bounds are derived from primitive geometry and local transforms,
    // so an update would otherwise leave them stale and let the builder cull
    // content that has moved back into view. Grown only once every transform
    // is updated, so each path up is composed with the new ones.
    for (id, dirty) in changed {
        match compiled.entity(*id) {
            CompiledEntity::Primitive(key)
                if dirty.intersects(RenderDirty::GEOMETRY | RenderDirty::PAINT) =>
            {
                let primitive = &compiled.primitives[key];
                let (spatial, bounds) = (primitive.spatial, primitive.primitive.paint_bounds());
                grow_subtree_bounds(compiled, Some(spatial), bounds);
            }
            CompiledEntity::Spatial(key) if dirty.contains(RenderDirty::GEOMETRY) => {
                let spatial = compiled.spatial_nodes[key];
                if let Some(bounds) = spatial.subtree_bounds {
                    let bounds = spatial.local_transform.transform_bounds(bounds);
                    grow_subtree_bounds(compiled, spatial.parent, bounds);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn empty_compiled_scene(source: &RenderScene) -> CompiledScene {
    let mut pictures = SlotMap::with_key();
    let mut spatial_nodes = SlotMap::with_key();
    let root_spatial = spatial_nodes.insert(CompiledSpatialNode {
        source: source.root(),
        parent: None,
        depth: 0,
        local_transform: Affine::IDENTITY,
        content_version: ContentVersion::default(),
        subtree_bounds: None,
    });
    let root_picture = pictures.insert(CompiledPicture {
        source: source.root(),
        items: Vec::new(),
        spans: Vec::new(),
        descriptor: LayerDescriptor::default(),
        render_program: None,
        placement_spatial: root_spatial,
        placement_clip: None,
        content_version: ContentVersion::default(),
        composite_version: 0,
        effect_version: 0,
        is_root: true,
    });
    let mut nodes = SecondaryMap::new();
    nodes.insert(
        source.root(),
        CompiledNode {
            layout: None,
            entity: CompiledEntity::Picture(root_picture),
        },
    );
    CompiledScene {
        root_picture,
        root_spatial,
        pictures,
        primitives: SlotMap::with_key(),
        spatial_nodes,
        clips: SlotMap::with_key(),
        nodes,
        spatial_token: super::fresh_token(),
        walk_generation: 0,
        scene_revision: 0,
        spare_items: Vec::new(),
        spare_spans: Vec::new(),
    }
}

/// Minimum number of items a span must cover to earn its per-frame bounds
/// test; below that the test costs about as much as building the items.
const MIN_SPAN_ITEMS: u32 = 4;

/// What one `walk_node` call contributed, reported in the *caller's* spatial
/// space so enclosing transforms can keep accumulating.
#[derive(Clone, Copy)]
struct WalkOutput {
    local_bounds: Option<Bounds>,
    /// False once the subtree emits an isolated picture: layer effects can
    /// paint outside the recorded bounds, so no enclosing span may cull it.
    boundable: bool,
}

impl WalkOutput {
    const EMPTY: Self = Self {
        local_bounds: None,
        boundable: true,
    };

    fn merge(self, other: Self) -> Self {
        Self {
            local_bounds: match (self.local_bounds, other.local_bounds) {
                (Some(a), Some(b)) => Some(a.union(b)),
                (Some(bounds), None) | (None, Some(bounds)) => Some(bounds),
                (None, None) => None,
            },
            boundable: self.boundable && other.boundable,
        }
    }
}

/// Drops spans too small to pay for themselves, then puts the rest in
/// pre-order so the builder meets an enclosing span before the ones nested in
/// it. Both steps are safe to skip: fewer spans only means coarser culling.
///
/// Nested transforms with nothing between them cover the same items. The
/// outer one goes first: it rejects with a spatial node fewer to resolve, and
/// when a dynamic transform on the inner one leaves it stale, the builder
/// moves on to the inner one, which stays valid.
fn finalize_picture_spans(
    picture: &mut CompiledPicture,
    spatial_nodes: &SlotMap<SpatialNodeId, CompiledSpatialNode>,
) {
    picture
        .spans
        .retain(|span| span.end - span.start >= MIN_SPAN_ITEMS);
    let spans = &mut picture.spans;
    spans.sort_unstable_by_key(|span| (span.start, std::cmp::Reverse(span.end)));
    // Spans over the same items now sit together; only those few need their
    // spatial nodes looked up to order them.
    let mut group = 0;
    while group < spans.len() {
        let same = |span: &CompiledItemSpan| {
            span.start == spans[group].start && span.end == spans[group].end
        };
        let end = group
            + 1
            + spans[group + 1..]
                .iter()
                .take_while(|span| same(span))
                .count();
        if end - group > 1 {
            spans[group..end].sort_unstable_by_key(|span| spatial_nodes[span.spatial].depth);
        }
        group = end;
    }
}

#[derive(Clone, Copy)]
struct WalkContext {
    picture: PictureId,
    spatial: SpatialNodeId,
    clip: Option<CompiledClipId>,
}

/// A structural rebuild that walks only the paths leading to a change.
///
/// A subtree with nothing dirty in it, still under the parent the last walk
/// placed it in, keeps its items: they are copied out of the previous
/// contents of its picture instead of being walked again. On the first
/// compile nothing has been placed, so the whole scene is walked. Entities of
/// removed nodes are dropped from the scene's removal log (see
/// [`SceneCompiler::compile`]) rather than by a sweep, so the cost follows
/// what changed, not the size of the scene. Entities of a subtree that is
/// detached or hidden but not removed stay until it is removed or walked again.
fn rebuild_dirty_paths(
    compiled: &mut CompiledScene,
    source: &RenderScene,
) -> Result<(), SceneCompileError> {
    let root_picture = &mut compiled.pictures[compiled.root_picture];
    let old = OldItems {
        items: std::mem::take(&mut root_picture.items),
        spans: std::mem::take(&mut root_picture.spans),
    };
    reset_root(compiled, source)?;
    let root_in_place = compiled.layout(source.root()).is_some();
    walk_node(
        compiled,
        source,
        source.root(),
        WalkContext {
            picture: compiled.root_picture,
            spatial: compiled.root_spatial,
            clip: None,
        },
        Placement {
            parent: None,
            parent_start: 0,
            old_start: root_in_place.then_some(0),
        },
        &old,
    )?;
    finalize_picture_spans(
        &mut compiled.pictures[compiled.root_picture],
        &compiled.spatial_nodes,
    );
    old.recycle(compiled);
    Ok(())
}

/// Empties the root picture into fresh buffers and refreshes its versions.
fn reset_root(compiled: &mut CompiledScene, source: &RenderScene) -> Result<(), SceneCompileError> {
    compiled.walk_generation += 1;
    let root_node = source
        .node(source.root())
        .ok_or(SceneError::MissingNode(source.root()))?;
    let (items, spans) = compiled.spare_buffers();
    let root = &mut compiled.pictures[compiled.root_picture];
    root.items = items;
    root.spans = spans;
    root.content_version = root_node.epochs.content_version();
    root.composite_version = root_node.epochs.composite;
    root.effect_version = root_node.epochs.effect;
    Ok(())
}

/// Drops the compiled entities built for nodes the scene has removed.
fn drop_removed_sources(compiled: &mut CompiledScene, removed: &[RenderNodeId]) {
    for id in removed {
        let Some(node) = compiled.nodes.remove(*id) else {
            continue;
        };
        match node.entity {
            CompiledEntity::None => {}
            CompiledEntity::Primitive(key) => {
                compiled.primitives.remove(key);
            }
            CompiledEntity::Spatial(key) => {
                compiled.spatial_nodes.remove(key);
            }
            CompiledEntity::Clip(key) => {
                compiled.clips.remove(key);
            }
            CompiledEntity::Picture(key) => {
                compiled.pictures.remove(key);
            }
        }
    }
}

/// Marks `id` and its ancestors as having moved geometry since they were
/// walked, so none of them is copied with its recorded bounds.
///
/// Stops at the first ancestor already marked: a walk validates a whole
/// subtree at once, so an invalid node never sits under a valid one.
fn invalidate_layout_bounds(compiled: &mut CompiledScene, source: &RenderScene, id: RenderNodeId) {
    let mut current = Some(id);
    while let Some(id) = current {
        match compiled
            .nodes
            .get_mut(id)
            .and_then(|node| node.layout.as_mut())
        {
            Some(layout) if layout.bounds_valid => layout.bounds_valid = false,
            _ => break,
        }
        current = source.node(id).and_then(|node| node.parent);
    }
}

/// A picture's items and spans as the previous walk left them.
#[derive(Default)]
struct OldItems {
    items: Vec<CompiledPictureItem>,
    spans: Vec<CompiledItemSpan>,
}

impl OldItems {
    /// Hands the buffers back once the walk is done copying from them, for
    /// the next walk to fill.
    fn recycle(self, compiled: &mut CompiledScene) {
        let Self {
            mut items,
            mut spans,
        } = self;
        if items.capacity() > 0 {
            items.clear();
            compiled.spare_items.push(items);
        }
        if spans.capacity() > 0 {
            spans.clear();
            compiled.spare_spans.push(spans);
        }
    }
}

impl CompiledScene {
    /// Empty item and span buffers for a picture about to be walked.
    fn spare_buffers(&mut self) -> (Vec<CompiledPictureItem>, Vec<CompiledItemSpan>) {
        (
            self.spare_items.pop().unwrap_or_default(),
            self.spare_spans.pop().unwrap_or_default(),
        )
    }
}

/// Where a node is being placed, and where its items were.
#[derive(Clone, Copy)]
struct Placement {
    parent: Option<RenderNodeId>,
    /// Index of the parent's first item in the current picture.
    parent_start: u32,
    /// Index of the node's first item in the picture's previous items, when
    /// the node sits where the last walk put it. `None` walks it afresh.
    old_start: Option<u32>,
}

/// Copies a clean subtree's previous items, and the spans inside them, to the
/// end of the picture. Returns `false`, copying nothing, when the recorded
/// range does not fit the previous items.
fn copy_previous_items(
    compiled: &mut CompiledScene,
    context: WalkContext,
    old: &OldItems,
    old_start: u32,
    count: u32,
) -> bool {
    let old_end = old_start + count;
    let items = &old.items[old_start as usize..old_end as usize];

    let picture = &mut compiled.pictures[context.picture];
    let new_start = picture.items.len() as u32;
    picture.items.extend_from_slice(items);

    #[cfg(test)]
    tests::COPIED_ITEMS.with(|copied| copied.set(copied.get() + items.len()));

    // A span covering exactly this range may belong to an ancestor transform
    // instead, which is being walked and records its own. The subtree's spans
    // all sit under spatial nodes deeper than the one it is placed in.
    let placed_depth = compiled.spatial_nodes[context.spatial].depth;
    let first = old.spans.partition_point(|span| span.start < old_start);
    for span in &old.spans[first..] {
        if span.start >= old_end {
            break;
        }
        let inside = span.end <= old_end
            && compiled
                .spatial_nodes
                .get(span.spatial)
                .is_some_and(|spatial| spatial.depth > placed_depth);
        if inside {
            picture.spans.push(CompiledItemSpan {
                start: span.start - old_start + new_start,
                end: span.end - old_start + new_start,
                ..*span
            });
        }
    }
    true
}

fn walk_node(
    compiled: &mut CompiledScene,
    source: &RenderScene,
    id: RenderNodeId,
    context: WalkContext,
    placement: Placement,
    old: &OldItems,
) -> Result<WalkOutput, SceneCompileError> {
    let node = source.node(id).ok_or(SceneError::MissingNode(id))?;
    let start = compiled.pictures[context.picture].items.len() as u32;
    let generation = compiled.walk_generation;
    let previous = compiled.layout(id).copied();
    let record = |compiled: &mut CompiledScene, output: WalkOutput| {
        let count = compiled.pictures[context.picture].items.len() as u32 - start;
        compiled.node_mut(id).layout = Some(NodeItemLayout {
            parent: placement.parent,
            offset: start - placement.parent_start,
            count,
            local_bounds: output.local_bounds,
            boundable: output.boundable,
            bounds_valid: true,
            generation,
            children_generation: generation,
        });
        output
    };

    // Nothing under the node changed since it was walked: its items are still
    // right, wherever they now land.
    if let Some(old_start) = placement.old_start
        && node.subtree_dirty.is_empty()
        && let Some(previous) = previous.filter(|layout| layout.bounds_valid)
        && copy_previous_items(compiled, context, old, old_start, previous.count)
    {
        compiled.node_mut(id).layout = Some(NodeItemLayout {
            offset: start - placement.parent_start,
            generation,
            ..previous
        });
        return Ok(WalkOutput {
            local_bounds: previous.local_bounds,
            boundable: previous.boundable,
        });
    }

    if !node.visible {
        return Ok(record(compiled, WalkOutput::EMPTY));
    }

    // Where a child's items were, if both it and this node are where the last
    // walk left them; a child is then copied or walked in turn. `old_base` is
    // where this node's items began in the previous items the child is
    // copied from: its own old start, or 0 inside its own picture.
    let child_placement = |compiled: &CompiledScene,
                           child: RenderNodeId,
                           parent_start: u32,
                           old_base: Option<u32>| {
        let old_start = old_base.zip(previous).and_then(|(old_base, previous)| {
            let child = compiled.layout(child)?;
            (child.parent == Some(id) && child.generation == previous.children_generation)
                .then_some(old_base + child.offset)
        });
        Placement {
            parent: Some(id),
            parent_start,
            old_start,
        }
    };
    // An entity written with the node's current content still holds it: every
    // edit of a primitive or clip bumps an epoch, so its clone can be skipped.
    let content_version = node.epochs.content_version();

    let output = match &node.kind {
        RenderNodeKind::Group(group) => {
            let mut output = WalkOutput::EMPTY;
            for child in &group.children {
                let placement = child_placement(compiled, *child, start, placement.old_start);
                output = output.merge(walk_node(
                    compiled, source, *child, context, placement, old,
                )?);
            }
            output
        }
        RenderNodeKind::Primitive(value) => {
            let primitive_id = match compiled.entity(id) {
                CompiledEntity::Primitive(key) => {
                    let primitive = &mut compiled.primitives[key];
                    if primitive.content_version != content_version {
                        primitive.primitive = value.primitive.clone();
                        primitive.content_version = content_version;
                    }
                    primitive.spatial = context.spatial;
                    primitive.clip = context.clip;
                    key
                }
                _ => {
                    let key = compiled.primitives.insert(CompiledPrimitive {
                        source: id,
                        primitive: value.primitive.clone(),
                        spatial: context.spatial,
                        clip: context.clip,
                        content_version,
                    });
                    compiled.node_mut(id).entity = CompiledEntity::Primitive(key);
                    key
                }
            };
            compiled.pictures[context.picture]
                .items
                .push(CompiledPictureItem::Primitive(primitive_id));
            WalkOutput {
                local_bounds: Some(value.primitive.paint_bounds()),
                boundable: true,
            }
        }
        RenderNodeKind::Transform(value) => {
            let spatial = CompiledSpatialNode {
                source: id,
                parent: Some(context.spatial),
                depth: compiled.spatial_nodes[context.spatial].depth + 1,
                local_transform: value.transform,
                content_version,
                subtree_bounds: None,
            };
            let spatial_id = match compiled.entity(id) {
                CompiledEntity::Spatial(key) => {
                    let old = &compiled.spatial_nodes[key];
                    if (old.parent, old.local_transform, old.content_version)
                        != (
                            spatial.parent,
                            spatial.local_transform,
                            spatial.content_version,
                        )
                    {
                        compiled.spatial_token = super::fresh_token();
                    }
                    compiled.spatial_nodes[key] = spatial;
                    key
                }
                _ => {
                    let key = compiled.spatial_nodes.insert(spatial);
                    compiled.node_mut(id).entity = CompiledEntity::Spatial(key);
                    key
                }
            };
            let Some(child) = value.child else {
                return Ok(record(compiled, WalkOutput::EMPTY));
            };
            let placement = child_placement(compiled, child, start, placement.old_start);
            let inner = walk_node(
                compiled,
                source,
                child,
                WalkContext {
                    spatial: spatial_id,
                    ..context
                },
                placement,
                old,
            )?;
            // `inner.local_bounds` is already in `spatial_id`'s local space,
            // which is exactly what the node records.
            let subtree_bounds = inner.local_bounds.filter(|_| inner.boundable);
            compiled.spatial_nodes[spatial_id].subtree_bounds = subtree_bounds;
            let picture = &mut compiled.pictures[context.picture];
            let end = picture.items.len() as u32;
            if subtree_bounds.is_some() {
                picture.spans.push(CompiledItemSpan {
                    spatial: spatial_id,
                    start,
                    end,
                });
            }
            WalkOutput {
                local_bounds: inner
                    .local_bounds
                    .map(|bounds| value.transform.transform_bounds(bounds)),
                boundable: inner.boundable,
            }
        }
        RenderNodeKind::Clip(value) => {
            let clip_id = match compiled.entity(id) {
                CompiledEntity::Clip(key) => {
                    let clip = &mut compiled.clips[key];
                    if clip.content_version != content_version {
                        clip.clip = value.clip.clone();
                        clip.content_version = content_version;
                    }
                    clip.parent = context.clip;
                    clip.spatial = context.spatial;
                    key
                }
                _ => {
                    let key = compiled.clips.insert(CompiledClip {
                        source: id,
                        parent: context.clip,
                        spatial: context.spatial,
                        clip: value.clip.clone(),
                        content_version,
                    });
                    compiled.node_mut(id).entity = CompiledEntity::Clip(key);
                    key
                }
            };
            let Some(child) = value.child else {
                return Ok(record(compiled, WalkOutput::EMPTY));
            };
            let placement = child_placement(compiled, child, start, placement.old_start);
            let inner = walk_node(
                compiled,
                source,
                child,
                WalkContext {
                    clip: Some(clip_id),
                    ..context
                },
                placement,
                old,
            )?;
            // The clip shares the caller's spatial node, so intersecting is a
            // valid tightening of the reported bounds.
            WalkOutput {
                local_bounds: inner
                    .local_bounds
                    .and_then(|bounds| bounds & value.clip.local_bounds()),
                boundable: inner.boundable,
            }
        }
        RenderNodeKind::Layer(value) if !value.descriptor.requires_isolation() => {
            // A layer that just lost isolation left its content in its own
            // picture, so that content is walked rather than copied.
            let was_isolated = match compiled.entity(id) {
                CompiledEntity::Picture(picture) => {
                    compiled.pictures.remove(picture);
                    compiled.node_mut(id).entity = CompiledEntity::None;
                    true
                }
                _ => false,
            };
            match value.child {
                Some(child) => {
                    let old_base = placement.old_start.filter(|_| !was_isolated);
                    let placement = child_placement(compiled, child, start, old_base);
                    walk_node(compiled, source, child, context, placement, old)?
                }
                None => WalkOutput::EMPTY,
            }
        }
        RenderNodeKind::Layer(value) => {
            let existing = match compiled.entity(id) {
                CompiledEntity::Picture(key) => Some(key),
                _ => None,
            };
            let render_program = match existing {
                Some(picture_id) => {
                    reusable_render_program(compiled, picture_id, id, &value.descriptor)?
                }
                None => compile_render_program(id, &value.descriptor)?,
            };
            // The content sits in the layer's own picture, so it can only be
            // copied from there, and only while the layer itself is in place.
            // The buffers are taken either way, to be refilled next time.
            let picture_old = match existing {
                Some(picture_id) => {
                    let picture = &mut compiled.pictures[picture_id];
                    OldItems {
                        items: std::mem::take(&mut picture.items),
                        spans: std::mem::take(&mut picture.spans),
                    }
                }
                None => OldItems::default(),
            };
            let (items, spans) = compiled.spare_buffers();
            let picture = CompiledPicture {
                source: id,
                items,
                spans,
                descriptor: value.descriptor.clone(),
                render_program: Some(render_program),
                placement_spatial: context.spatial,
                placement_clip: context.clip,
                content_version,
                composite_version: node.epochs.composite,
                effect_version: node.epochs.effect,
                is_root: false,
            };
            let picture_id = match existing {
                Some(key) => {
                    compiled.pictures[key] = picture;
                    key
                }
                None => {
                    let key = compiled.pictures.insert(picture);
                    compiled.node_mut(id).entity = CompiledEntity::Picture(key);
                    key
                }
            };
            compiled.pictures[context.picture]
                .items
                .push(CompiledPictureItem::Picture(picture_id));
            if let Some(child) = value.child {
                let old_base = existing.and(placement.old_start).map(|_| 0);
                let placement = child_placement(compiled, child, 0, old_base);
                walk_node(
                    compiled,
                    source,
                    child,
                    WalkContext {
                        picture: picture_id,
                        clip: None,
                        ..context
                    },
                    placement,
                    &picture_old,
                )?;
            }
            picture_old.recycle(compiled);
            finalize_picture_spans(&mut compiled.pictures[picture_id], &compiled.spatial_nodes);
            // An isolated layer's effects can paint outside its content, so
            // enclosing spans must not try to bound it.
            WalkOutput {
                local_bounds: None,
                boundable: false,
            }
        }
    };
    Ok(record(compiled, output))
}

fn compile_render_program(
    source: RenderNodeId,
    descriptor: &LayerDescriptor,
) -> Result<BuiltLayerProgram, SceneCompileError> {
    let program = descriptor
        .compile_render_program()
        .map(Arc::new)
        .map_err(|error| SceneCompileError::RenderGraph { source, error })?;
    descriptor
        .bind_render_program(program)
        .map_err(|error| SceneCompileError::RenderGraphBinding { source, error })
}

fn reusable_render_program(
    compiled: &CompiledScene,
    picture: PictureId,
    source: RenderNodeId,
    descriptor: &LayerDescriptor,
) -> Result<BuiltLayerProgram, SceneCompileError> {
    if let Some(existing) = compiled.pictures.get(picture)
        && existing.descriptor.has_same_render_graph_style(descriptor)
        && let Some(program) = &existing.render_program
    {
        let bindings = descriptor
            .render_graph_bindings()
            .map_err(|error| SceneCompileError::RenderGraphBinding { source, error })?;
        if program.bindings() == &bindings {
            return Ok(program.clone());
        }
        return xui_render_graph::BoundLayerProgram::new(Arc::clone(program.program()), bindings)
            .map_err(|error| SceneCompileError::RenderGraphBinding { source, error });
    }
    compile_render_program(source, descriptor)
}

fn validate_explicit_cache_keys(source: &RenderScene) -> Result<(), SceneCompileError> {
    fn visit(
        source: &RenderScene,
        id: RenderNodeId,
        keys: &mut HashSet<LayerCacheKey>,
    ) -> Result<(), SceneCompileError> {
        let node = source.node(id).ok_or(SceneError::MissingNode(id))?;
        if let RenderNodeKind::Layer(layer) = &node.kind
            && layer.descriptor.requires_isolation()
            && let Some(key) = layer.descriptor.cache_key
            && !keys.insert(key)
        {
            return Err(SceneCompileError::DuplicateLayerCacheKey(key));
        }
        for child in node.children() {
            visit(source, *child, keys)?;
        }
        Ok(())
    }

    visit(source, source.root(), &mut HashSet::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{
        BlendMode, BuiltFrame, BuiltItem, CachePolicy, ClipShape, CompositeOperator,
        CompositeStyle, FrameBuilder, FrameProperties, Primitive, Shape, ShapePrimitive,
    };
    use std::sync::Arc;
    use xui_interface::{
        Bounds, Color, ComputedColorStyle, ComputedEffect, FilterQuality, ImageData, ImageKey, Size,
    };

    fn shape(rect: Bounds, color: Color) -> Primitive {
        Primitive::Shape(ShapePrimitive {
            bounds: rect,
            shape: Shape::Rect,
            fill: Some(ComputedColorStyle::Solid(color)),
            stroke: None,
            shadow: None,
        })
    }

    #[test]
    fn primitive_and_spatial_ids_survive_non_structural_updates() {
        let mut source = RenderScene::new();
        let transform = source.insert_transform(Affine::IDENTITY);
        let primitive = source.insert_primitive(shape(
            Bounds::from_origin_size((0.0, 0.0), (10.0, 10.0)),
            Color::BLACK,
        ));
        source.append_child(source.root(), transform).unwrap();
        source.set_child(transform, Some(primitive)).unwrap();

        let mut compiler = SceneCompiler::new();
        let first_snapshot = source.dirty_snapshot();
        let first = compiler.compile(&source, &first_snapshot).unwrap();
        let first_primitive = first.primitive_for_source(primitive).unwrap();
        let first_spatial = first.spatial_for_source(transform).unwrap();

        source
            .update_transform(transform, Affine::translate(5.0, 7.0))
            .unwrap();
        source
            .update_primitive(
                primitive,
                shape(
                    Bounds::from_origin_size((0.0, 0.0), (20.0, 10.0)),
                    Color::WHITE,
                ),
            )
            .unwrap();
        let second_snapshot = source.dirty_snapshot();
        let second = compiler.compile(&source, &second_snapshot).unwrap();

        assert_eq!(
            second.primitive_for_source(primitive).unwrap(),
            first_primitive
        );
        assert_eq!(second.spatial_for_source(transform).unwrap(), first_spatial);
        assert_eq!(
            second.spatial_nodes[first_spatial].local_transform,
            Affine::translate(5.0, 7.0)
        );
    }

    #[test]
    fn repeated_unacknowledged_snapshot_is_idempotent() {
        let mut source = RenderScene::new();
        let primitive = source.insert_primitive(shape(
            Bounds::from_origin_size((0.0, 0.0), (10.0, 10.0)),
            Color::BLACK,
        ));
        source.append_child(source.root(), primitive).unwrap();
        let snapshot = source.dirty_snapshot();
        let mut compiler = SceneCompiler::new();
        let first_revision = compiler
            .compile(&source, &snapshot)
            .unwrap()
            .scene_revision();
        let first_id = compiler
            .compiled_scene()
            .unwrap()
            .primitive_for_source(primitive)
            .unwrap();
        let second = compiler.compile(&source, &snapshot).unwrap();
        assert_eq!(second.scene_revision(), first_revision);
        assert_eq!(second.primitive_for_source(primitive).unwrap(), first_id);
    }

    #[test]
    fn topology_rebuild_reuses_surviving_ids_and_removes_stale_items() {
        let mut source = RenderScene::new();
        let first = source.insert_primitive(shape(
            Bounds::from_origin_size((0.0, 0.0), (10.0, 10.0)),
            Color::BLACK,
        ));
        let stale = source.insert_primitive(shape(
            Bounds::from_origin_size((20.0, 0.0), (10.0, 10.0)),
            Color::WHITE,
        ));
        source.append_child(source.root(), first).unwrap();
        source.append_child(source.root(), stale).unwrap();
        let mut compiler = SceneCompiler::new();
        let snapshot = source.dirty_snapshot();
        let first_id = compiler
            .compile(&source, &snapshot)
            .unwrap()
            .primitive_for_source(first)
            .unwrap();

        source.remove_subtree(stale).unwrap();
        let snapshot = source.dirty_snapshot();
        let compiled = compiler.compile(&source, &snapshot).unwrap();
        assert_eq!(compiled.primitive_for_source(first).unwrap(), first_id);
        assert_eq!(compiled.primitive_for_source(stale), None);
        assert_eq!(compiled.pictures[compiled.root_picture].items.len(), 1);
    }

    #[test]
    fn removing_an_already_detached_subtree_drops_its_entities() {
        let mut source = RenderScene::new();
        let transform = source.insert_transform(Affine::translate(1.0, 2.0));
        let primitive = source.insert_primitive(shape(
            Bounds::from_origin_size((0.0, 0.0), (10.0, 10.0)),
            Color::BLACK,
        ));
        source.append_child(source.root(), transform).unwrap();
        source.set_child(transform, Some(primitive)).unwrap();
        let mut compiler = SceneCompiler::new();
        let compile = |compiler: &mut SceneCompiler, source: &mut RenderScene| {
            let snapshot = source.dirty_snapshot();
            compiler.compile(source, &snapshot).unwrap();
            source.acknowledge(&snapshot);
        };
        compile(&mut compiler, &mut source);

        // Detached but alive: kept, in case it comes back.
        source.detach(transform).unwrap();
        compile(&mut compiler, &mut source);
        let compiled = compiler.compiled_scene().unwrap();
        assert!(compiled.spatial_for_source(transform).is_some());
        assert!(compiled.pictures[compiled.root_picture].items.is_empty());

        // Removing it dirties nothing the scene still holds.
        source.remove_subtree(transform).unwrap();
        assert!(!source.is_dirty());
        compile(&mut compiler, &mut source);
        let compiled = compiler.compiled_scene().unwrap();
        assert_eq!(compiled.spatial_for_source(transform), None);
        assert_eq!(compiled.primitive_for_source(primitive), None);
        assert!(compiled.primitives.is_empty());
        assert_eq!(
            compiled.spatial_nodes.len(),
            1,
            "only the root spatial node"
        );
    }

    #[test]
    fn clip_and_composite_updates_keep_compiled_ids() {
        let mut source = RenderScene::new();
        let clip = source.insert_clip(ClipShape::Rect(Bounds::from_origin_size(
            (0.0, 0.0),
            (10.0, 10.0),
        )));
        let layer = source.insert_layer(LayerDescriptor {
            force_offscreen: true,
            ..LayerDescriptor::default()
        });
        source.append_child(source.root(), clip).unwrap();
        source.set_child(clip, Some(layer)).unwrap();
        let mut compiler = SceneCompiler::new();
        let snapshot = source.dirty_snapshot();
        let first = compiler.compile(&source, &snapshot).unwrap();
        let clip_id = first.clip_for_source(clip).unwrap();
        let picture_id = first.picture_for_source(layer).unwrap();

        source
            .update_clip(
                clip,
                ClipShape::Rect(Bounds::from_origin_size((1.0, 2.0), (20.0, 30.0))),
            )
            .unwrap();
        source
            .update_layer_composite(
                layer,
                CompositeStyle {
                    opacity: 0.5,
                    ..CompositeStyle::default()
                },
            )
            .unwrap();
        let snapshot = source.dirty_snapshot();
        let second = compiler.compile(&source, &snapshot).unwrap();
        assert_eq!(second.clip_for_source(clip), Some(clip_id));
        assert_eq!(second.picture_for_source(layer), Some(picture_id));
        assert_eq!(
            second.clip(clip_id).unwrap().clip,
            ClipShape::Rect(Bounds::from_origin_size((1.0, 2.0), (20.0, 30.0)))
        );
        assert_eq!(
            second
                .picture(picture_id)
                .unwrap()
                .descriptor
                .composite
                .opacity,
            0.5
        );
    }

    #[test]
    fn isolation_switch_adds_and_removes_picture_without_replacing_primitive() {
        let mut source = RenderScene::new();
        let layer = source.insert_layer(LayerDescriptor::default());
        let primitive = source.insert_primitive(shape(
            Bounds::from_origin_size((0.0, 0.0), (10.0, 10.0)),
            Color::BLACK,
        ));
        source.append_child(source.root(), layer).unwrap();
        source.set_child(layer, Some(primitive)).unwrap();
        let mut compiler = SceneCompiler::new();
        let snapshot = source.dirty_snapshot();
        let first = compiler.compile(&source, &snapshot).unwrap();
        let primitive_id = first.primitive_for_source(primitive).unwrap();
        assert_eq!(first.picture_for_source(layer), None);

        source
            .update_layer_descriptor(
                layer,
                LayerDescriptor {
                    force_offscreen: true,
                    ..LayerDescriptor::default()
                },
            )
            .unwrap();
        let snapshot = source.dirty_snapshot();
        let isolated = compiler.compile(&source, &snapshot).unwrap();
        assert!(isolated.picture_for_source(layer).is_some());
        assert_eq!(isolated.primitive_for_source(primitive), Some(primitive_id));

        source
            .update_layer_descriptor(layer, LayerDescriptor::default())
            .unwrap();
        let snapshot = source.dirty_snapshot();
        let flattened = compiler.compile(&source, &snapshot).unwrap();
        assert_eq!(flattened.picture_for_source(layer), None);
        assert_eq!(
            flattened.primitive_for_source(primitive),
            Some(primitive_id)
        );
    }

    #[test]
    fn topology_inside_an_isolated_layer_rebuilds_that_picture() {
        let mut source = RenderScene::new();
        let layer = source.insert_layer(LayerDescriptor {
            force_offscreen: true,
            ..LayerDescriptor::default()
        });
        let group = source.insert_group();
        let first = source.insert_primitive(shape(
            Bounds::from_origin_size((0.0, 0.0), (10.0, 10.0)),
            Color::BLACK,
        ));
        source.append_child(source.root(), layer).unwrap();
        source.set_child(layer, Some(group)).unwrap();
        source.append_child(group, first).unwrap();
        let mut compiler = SceneCompiler::new();
        let snapshot = source.dirty_snapshot();
        compiler.compile(&source, &snapshot).unwrap();

        let second = source.insert_primitive(shape(
            Bounds::from_origin_size((20.0, 0.0), (10.0, 10.0)),
            Color::WHITE,
        ));
        source.append_child(group, second).unwrap();

        let snapshot = source.dirty_snapshot();
        let compiled = compiler.compile(&source, &snapshot).unwrap();
        let picture = compiled.picture_for_source(layer).unwrap();
        assert_eq!(compiled.picture(picture).unwrap().items.len(), 2);
    }

    #[test]
    fn reorder_updates_picture_order_without_replacing_primitives() {
        let mut source = RenderScene::new();
        let first = source.insert_primitive(shape(
            Bounds::from_origin_size((0.0, 0.0), (10.0, 10.0)),
            Color::BLACK,
        ));
        let second = source.insert_primitive(shape(
            Bounds::from_origin_size((20.0, 0.0), (10.0, 10.0)),
            Color::WHITE,
        ));
        source.append_child(source.root(), first).unwrap();
        source.append_child(source.root(), second).unwrap();
        let mut compiler = SceneCompiler::new();
        let snapshot = source.dirty_snapshot();
        let initial = compiler.compile(&source, &snapshot).unwrap();
        let first_id = initial.primitive_for_source(first).unwrap();
        let second_id = initial.primitive_for_source(second).unwrap();

        source.reorder_child(source.root(), second, 0).unwrap();
        let snapshot = source.dirty_snapshot();
        let reordered = compiler.compile(&source, &snapshot).unwrap();
        assert_eq!(
            reordered.picture(reordered.root_picture()).unwrap().items,
            vec![
                CompiledPictureItem::Primitive(second_id),
                CompiledPictureItem::Primitive(first_id),
            ]
        );
    }

    #[test]
    fn duplicate_cache_key_failure_keeps_previous_scene() {
        let mut source = RenderScene::new();
        let mut keys = SlotMap::<LayerCacheKey, ()>::with_key();
        let key = keys.insert(());
        let first = source.insert_layer(LayerDescriptor {
            cache_key: Some(key),
            cache_policy: CachePolicy::Always,
            ..LayerDescriptor::default()
        });
        source.append_child(source.root(), first).unwrap();
        let mut compiler = SceneCompiler::new();
        let snapshot = source.dirty_snapshot();
        let revision = compiler
            .compile(&source, &snapshot)
            .unwrap()
            .scene_revision();

        let second = source.insert_layer(LayerDescriptor {
            cache_key: Some(key),
            cache_policy: CachePolicy::Always,
            ..LayerDescriptor::default()
        });
        source.append_child(source.root(), second).unwrap();
        let snapshot = source.dirty_snapshot();
        assert!(matches!(
            compiler.compile(&source, &snapshot),
            Err(SceneCompileError::DuplicateLayerCacheKey(found)) if found == key
        ));
        assert_eq!(
            compiler.compiled_scene().unwrap().scene_revision(),
            revision
        );
    }

    #[test]
    fn isolated_picture_compiles_and_refreshes_static_render_program() {
        let mut source = RenderScene::new();
        let effects: Arc<[ComputedEffect]> = Arc::from([ComputedEffect::Blur {
            sigma_x: 2.0,
            sigma_y: 2.0,
            quality: FilterQuality::Medium,
        }]);
        let layer = source.insert_layer(LayerDescriptor {
            effects: effects.clone(),
            force_offscreen: true,
            ..LayerDescriptor::default()
        });
        let contents = source.insert_group();
        source.append_child(source.root(), layer).unwrap();
        source.set_child(layer, Some(contents)).unwrap();
        let mut compiler = SceneCompiler::new();
        let first = compiler.compile(&source, &source.dirty_snapshot()).unwrap();
        let picture_id = first.picture_for_source(layer).unwrap();
        let first_program = first
            .picture(picture_id)
            .unwrap()
            .render_program
            .as_ref()
            .unwrap()
            .clone();
        let first_fingerprint = first_program.program().fingerprint();

        let primitive = source.insert_primitive(shape(
            Bounds::from_origin_size((0.0, 0.0), (10.0, 10.0)),
            Color::BLACK,
        ));
        source.append_child(contents, primitive).unwrap();
        let topology_update = compiler.compile(&source, &source.dirty_snapshot()).unwrap();
        assert!(Arc::ptr_eq(
            first_program.program(),
            topology_update
                .picture(picture_id)
                .unwrap()
                .render_program
                .as_ref()
                .unwrap()
                .program()
        ));

        source
            .update_layer_composite(
                layer,
                CompositeStyle {
                    opacity: 0.5,
                    transform: Affine::translate(5.0, 0.0),
                    ..CompositeStyle::default()
                },
            )
            .unwrap();
        let dynamic_style_update = compiler.compile(&source, &source.dirty_snapshot()).unwrap();
        assert!(Arc::ptr_eq(
            first_program.program(),
            dynamic_style_update
                .picture(picture_id)
                .unwrap()
                .render_program
                .as_ref()
                .unwrap()
                .program()
        ));

        source
            .update_layer_descriptor(
                layer,
                LayerDescriptor {
                    effects,
                    force_offscreen: true,
                    composite: CompositeStyle {
                        blend_mode: BlendMode::Multiply,
                        operator: CompositeOperator::SrcOver,
                        ..CompositeStyle::default()
                    },
                    ..LayerDescriptor::default()
                },
            )
            .unwrap();
        let second = compiler.compile(&source, &source.dirty_snapshot()).unwrap();
        let second_program = second
            .picture(picture_id)
            .unwrap()
            .render_program
            .as_ref()
            .unwrap();
        assert_ne!(first_fingerprint, second_program.program().fingerprint());
        assert!(second_program.program().nodes().iter().any(|node| matches!(
            node.op,
            xui_render_graph::ProgramOp::LayerComposite {
                blend_mode: BlendMode::Multiply,
                ..
            }
        )));
    }

    #[test]
    fn mask_resource_changes_rebind_without_recompiling_ir() {
        fn mask(key: u64, value: u8) -> ComputedEffect {
            ComputedEffect::ImageMask {
                image: ImageKey::UserProvided(key),
                data: ImageData::rgba8(Size::new(1, 1), vec![value; 4]),
                bounds: Bounds::from_origin_size((0.0, 0.0), (10.0, 10.0)),
            }
        }

        let mut source = RenderScene::new();
        let layer = source.insert_layer(LayerDescriptor {
            effects: Arc::from([mask(1, 1)]),
            force_offscreen: true,
            ..LayerDescriptor::default()
        });
        source.append_child(source.root(), layer).unwrap();
        let mut compiler = SceneCompiler::new();
        let first = compiler.compile(&source, &source.dirty_snapshot()).unwrap();
        let picture_id = first.picture_for_source(layer).unwrap();
        let first = first
            .picture(picture_id)
            .unwrap()
            .render_program
            .clone()
            .unwrap();

        source
            .update_layer_descriptor(
                layer,
                LayerDescriptor {
                    effects: Arc::from([mask(2, 2)]),
                    force_offscreen: true,
                    ..LayerDescriptor::default()
                },
            )
            .unwrap();
        let second = compiler.compile(&source, &source.dirty_snapshot()).unwrap();
        let second = second
            .picture(picture_id)
            .unwrap()
            .render_program
            .as_ref()
            .unwrap();

        assert!(Arc::ptr_eq(first.program(), second.program()));
        assert!(matches!(
            second.handle(xui_render_graph::ExternalResourceKind::LayerMask(0)),
            Some(crate::render::render_graph::ImageResource::Data {
                key: ImageKey::UserProvided(2),
                ..
            })
        ));
    }

    #[test]
    fn invalid_layer_style_reports_source_from_scene_compiler() {
        let mut source = RenderScene::new();
        let layer = source.insert_layer(LayerDescriptor {
            effects: Arc::from([ComputedEffect::Blur {
                sigma_x: f32::NAN,
                sigma_y: f32::NAN,
                quality: FilterQuality::Medium,
            }]),
            force_offscreen: true,
            ..LayerDescriptor::default()
        });
        source.append_child(source.root(), layer).unwrap();
        let mut compiler = SceneCompiler::new();
        assert!(matches!(
            compiler.compile(&source, &source.dirty_snapshot()),
            Err(SceneCompileError::RenderGraph { source, .. }) if source == layer
        ));
    }

    thread_local! {
        pub(super) static COPIED_ITEMS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// A compiled scene with every entity named by its source node, so a scene
    /// compiled incrementally can be compared with one compiled from scratch.
    #[derive(Debug, PartialEq)]
    struct CanonicalPicture {
        source: RenderNodeId,
        placement: Vec<(RenderNodeId, Affine, u32)>,
        placement_clip: Vec<(RenderNodeId, ClipShape, RenderNodeId)>,
        items: Vec<CanonicalItem>,
        spans: Vec<(u32, u32, RenderNodeId)>,
    }

    #[derive(Debug, PartialEq)]
    enum CanonicalItem {
        Primitive {
            source: RenderNodeId,
            primitive: Primitive,
            spatial: Vec<(RenderNodeId, Affine, u32)>,
            clip: Vec<(RenderNodeId, ClipShape, RenderNodeId)>,
            content_version: ContentVersion,
        },
        Picture(CanonicalPicture),
    }

    fn spatial_chain(
        compiled: &CompiledScene,
        mut id: Option<SpatialNodeId>,
    ) -> Vec<(RenderNodeId, Affine, u32)> {
        let mut chain = Vec::new();
        while let Some(spatial) = id.map(|id| &compiled.spatial_nodes[id]) {
            chain.push((spatial.source, spatial.local_transform, spatial.depth));
            id = spatial.parent;
        }
        chain
    }

    fn clip_chain(
        compiled: &CompiledScene,
        mut id: Option<CompiledClipId>,
    ) -> Vec<(RenderNodeId, ClipShape, RenderNodeId)> {
        let mut chain = Vec::new();
        while let Some(clip) = id.map(|id| &compiled.clips[id]) {
            let spatial = compiled.spatial_nodes[clip.spatial].source;
            chain.push((clip.source, clip.clip.clone(), spatial));
            id = clip.parent;
        }
        chain
    }

    /// Span bounds are left out of the canonical form and collected into
    /// `bounds` instead: an in-place geometry update refreshes them without
    /// re-applying clips, so they may legitimately be larger than a fresh
    /// compile's.
    fn canonical(
        compiled: &CompiledScene,
        id: PictureId,
        bounds: &mut Vec<Bounds>,
    ) -> CanonicalPicture {
        let picture = &compiled.pictures[id];
        bounds.extend(
            picture
                .spans
                .iter()
                .filter_map(|span| compiled.spatial_nodes[span.spatial].subtree_bounds),
        );
        CanonicalPicture {
            source: picture.source,
            placement: spatial_chain(compiled, Some(picture.placement_spatial)),
            placement_clip: clip_chain(compiled, picture.placement_clip),
            items: picture
                .items
                .iter()
                .map(|item| match item {
                    CompiledPictureItem::Primitive(id) => {
                        let primitive = &compiled.primitives[*id];
                        CanonicalItem::Primitive {
                            source: primitive.source,
                            primitive: primitive.primitive.clone(),
                            spatial: spatial_chain(compiled, Some(primitive.spatial)),
                            clip: clip_chain(compiled, primitive.clip),
                            content_version: primitive.content_version,
                        }
                    }
                    CompiledPictureItem::Picture(id) => {
                        CanonicalItem::Picture(canonical(compiled, *id, bounds))
                    }
                })
                .collect(),
            spans: picture
                .spans
                .iter()
                .map(|span| {
                    let spatial = compiled.spatial_nodes[span.spatial].source;
                    (span.start, span.end, spatial)
                })
                .collect(),
        }
    }

    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n as u64) as usize
        }

        fn coordinate(&mut self) -> f32 {
            self.below(200) as f32
        }
    }

    fn is_inside(source: &RenderScene, node: RenderNodeId, ancestor: RenderNodeId) -> bool {
        let mut current = Some(node);
        while let Some(id) = current {
            if id == ancestor {
                return true;
            }
            current = source.node(id).and_then(|node| node.parent);
        }
        false
    }

    /// One random edit through the scene's public API. Every edit is valid, so
    /// none of them may fail.
    fn random_edit(source: &mut RenderScene, nodes: &mut Vec<RenderNodeId>, rng: &mut Rng) {
        nodes.retain(|id| source.contains(*id));
        let of_kind =
            |source: &RenderScene, nodes: &[RenderNodeId], kind: fn(&RenderNodeKind) -> bool| {
                nodes
                    .iter()
                    .copied()
                    .filter(|id| kind(&source.node(*id).unwrap().kind))
                    .collect::<Vec<_>>()
            };
        let groups = of_kind(source, nodes, |kind| {
            matches!(kind, RenderNodeKind::Group(_))
        });
        let parent = groups[rng.below(groups.len())];
        let bounds = |rng: &mut Rng| {
            Bounds::from_origin_size((rng.coordinate(), rng.coordinate()), (10.0, 10.0))
        };
        // A new subtree, attached under `parent`, behind a wrapper of some kind.
        let wrap =
            |source: &mut RenderScene, nodes: &mut Vec<RenderNodeId>, wrapper: RenderNodeId| {
                let inner = source.insert_group();
                source.set_child(wrapper, Some(inner)).unwrap();
                source.append_child(parent, wrapper).unwrap();
                nodes.extend([wrapper, inner]);
            };
        let removable: Vec<_> = nodes
            .iter()
            .copied()
            .filter(|id| *id != source.root())
            .collect();
        let wrappers = of_kind(source, nodes, |kind| {
            matches!(
                kind,
                RenderNodeKind::Transform(_) | RenderNodeKind::Clip(_) | RenderNodeKind::Layer(_)
            )
        });
        match rng.below(18) {
            0 | 1 => {
                let primitive = source.insert_primitive(shape(bounds(rng), Color::BLACK));
                let index = rng.below(source.children(parent).unwrap().len() + 1);
                source.insert_child(parent, index, primitive).unwrap();
                nodes.push(primitive);
            }
            2 => {
                let group = source.insert_group();
                source.append_child(parent, group).unwrap();
                nodes.push(group);
            }
            3 => {
                let transform =
                    source.insert_transform(Affine::translate(rng.coordinate(), rng.coordinate()));
                wrap(source, nodes, transform);
            }
            4 => {
                let clip = source.insert_clip(ClipShape::Rect(bounds(rng)));
                wrap(source, nodes, clip);
            }
            5 => {
                let layer = source.insert_layer(LayerDescriptor {
                    force_offscreen: rng.below(2) == 0,
                    ..LayerDescriptor::default()
                });
                wrap(source, nodes, layer);
            }
            6 if !removable.is_empty() => {
                source
                    .remove_subtree(removable[rng.below(removable.len())])
                    .unwrap();
            }
            7 if !removable.is_empty() => {
                source
                    .detach(removable[rng.below(removable.len())])
                    .unwrap();
            }
            8 => {
                let detached: Vec<_> = removable
                    .iter()
                    .copied()
                    .filter(|id| source.node(*id).unwrap().parent.is_none())
                    .filter(|id| !is_inside(source, parent, *id))
                    .collect();
                if !detached.is_empty() {
                    let node = detached[rng.below(detached.len())];
                    source.append_child(parent, node).unwrap();
                }
            }
            9 if !removable.is_empty() => {
                let node = removable[rng.below(removable.len())];
                let visible = source.node(node).unwrap().visible;
                source.set_visible(node, !visible).unwrap();
            }
            10 => {
                let transforms = of_kind(source, nodes, |kind| {
                    matches!(kind, RenderNodeKind::Transform(_))
                });
                if !transforms.is_empty() {
                    let transform = Affine::translate(rng.coordinate(), rng.coordinate());
                    source
                        .update_transform(transforms[rng.below(transforms.len())], transform)
                        .unwrap();
                }
            }
            11 => {
                let primitives = of_kind(source, nodes, |kind| {
                    matches!(kind, RenderNodeKind::Primitive(_))
                });
                if !primitives.is_empty() {
                    let primitive = shape(bounds(rng), Color::WHITE);
                    source
                        .update_primitive(primitives[rng.below(primitives.len())], primitive)
                        .unwrap();
                }
            }
            12 => {
                let layers = of_kind(source, nodes, |kind| {
                    matches!(kind, RenderNodeKind::Layer(_))
                });
                if !layers.is_empty() {
                    let layer = layers[rng.below(layers.len())];
                    let RenderNodeKind::Layer(value) = &source.node(layer).unwrap().kind else {
                        unreachable!()
                    };
                    let descriptor = LayerDescriptor {
                        force_offscreen: !value.descriptor.force_offscreen,
                        ..value.descriptor.clone()
                    };
                    source.update_layer_descriptor(layer, descriptor).unwrap();
                }
            }
            14 if !wrappers.is_empty() => {
                // Swap a wrapper's child for a fresh group, or for nothing.
                let wrapper = wrappers[rng.below(wrappers.len())];
                let replacement = (rng.below(3) > 0).then(|| source.insert_group());
                nodes.extend(replacement);
                source.set_child(wrapper, replacement).unwrap();
            }
            15 => {
                let clips = of_kind(source, nodes, |kind| {
                    matches!(kind, RenderNodeKind::Clip(_))
                });
                if !clips.is_empty() {
                    let clip = ClipShape::Rect(bounds(rng));
                    source
                        .update_clip(clips[rng.below(clips.len())], clip)
                        .unwrap();
                }
            }
            16 => {
                let layers = of_kind(source, nodes, |kind| {
                    matches!(kind, RenderNodeKind::Layer(_))
                });
                if !layers.is_empty() {
                    let composite = CompositeStyle {
                        opacity: rng.below(10) as f32 / 10.0,
                        ..CompositeStyle::default()
                    };
                    source
                        .update_layer_composite(layers[rng.below(layers.len())], composite)
                        .unwrap();
                }
            }
            17 => {
                let children = source.children(parent).unwrap();
                if children.len() > 1 {
                    let child = children[rng.below(children.len())];
                    let index = rng.below(children.len());
                    source.reorder_child(parent, child, index).unwrap();
                }
            }
            _ => {
                let mut children = source.children(parent).unwrap().to_vec();
                for index in (1..children.len()).rev() {
                    children.swap(index, rng.below(index + 1));
                }
                source.set_children(parent, &children).unwrap();
            }
        }
    }

    /// Sets, changes or clears a dynamic transform on a transform node now and
    /// then, attached or not.
    fn random_dynamic_transform(
        source: &RenderScene,
        nodes: &[RenderNodeId],
        properties: &mut FrameProperties,
        rng: &mut Rng,
    ) {
        if rng.below(3) != 0 {
            return;
        }
        let transforms: Vec<_> = nodes
            .iter()
            .copied()
            .filter(|id| {
                source
                    .node(*id)
                    .is_some_and(|node| matches!(node.kind, RenderNodeKind::Transform(_)))
            })
            .collect();
        if transforms.is_empty() {
            return;
        }
        let transform = transforms[rng.below(transforms.len())];
        if rng.below(3) == 0 {
            properties.clear_transform(transform);
        } else {
            let to = Affine::translate(rng.coordinate(), rng.coordinate());
            properties.set_transform(transform, to);
        }
    }

    /// Everything a frame places, in order, so two builds can be compared.
    fn frame_digest(frame: &BuiltFrame) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        for layer in &frame.layers {
            write!(
                out,
                "L{:?}{:?}{:?}{:?}[",
                layer.source, layer.content_bounds, layer.render_bounds, layer.content_version
            )
            .unwrap();
            for item in &layer.items {
                match item {
                    BuiltItem::Draw(draw) => {
                        let common = draw.common();
                        write!(
                            out,
                            "D{:?}{:?}{:?}{:?}{:?};",
                            common.source,
                            common.content_version,
                            common.world_transform,
                            common.world_bounds,
                            common.clip_chain
                        )
                        .unwrap();
                    }
                    BuiltItem::Layer(id) => {
                        let instance = &frame.layer_instances[id.0];
                        write!(
                            out,
                            "I{:?}{:?}{:?}{:?}{:?};",
                            instance.source,
                            instance.layer,
                            instance.world_bounds,
                            instance.placement_version,
                            instance.clip_chain
                        )
                        .unwrap();
                    }
                }
            }
            out.push(']');
        }
        for clip in &frame.clip_chains {
            write!(
                out,
                "C{:?}{:?}{:?}{:?};",
                clip.source, clip.parent, clip.world_transform, clip.world_bounds
            )
            .unwrap();
        }
        out
    }

    #[test]
    fn incremental_compiles_match_a_compile_from_scratch() {
        COPIED_ITEMS.with(|copied| copied.set(0));
        for seed in 1..=24u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let mut source = RenderScene::new();
            let mut nodes = vec![source.root()];
            let mut compiler = SceneCompiler::new();
            // One builder across every step, reusing world transforms while
            // nothing moves, held against a fresh one each step.
            let mut builder = FrameBuilder::new();
            let mut properties = FrameProperties::default();
            let mut dynamic = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03));
            for step in 0..150 {
                for _ in 0..=rng.below(3) {
                    random_edit(&mut source, &mut nodes, &mut rng);
                }
                random_dynamic_transform(&source, &nodes, &mut properties, &mut dynamic);
                let snapshot = source.dirty_snapshot();
                let incremental = compiler.compile(&source, &snapshot).unwrap();
                let mut incremental_bounds = Vec::new();
                let incremental_picture = canonical(
                    incremental,
                    incremental.root_picture,
                    &mut incremental_bounds,
                );
                let viewport = Bounds::from_origin_size((0.0, 0.0), (100.0, 100.0));
                let reused = builder.build(incremental, viewport, &properties).unwrap();
                let rebuilt = FrameBuilder::new()
                    .build(incremental, viewport, &properties)
                    .unwrap();
                assert_eq!(
                    frame_digest(&reused),
                    frame_digest(&rebuilt),
                    "seed {seed}, step {step}: a reused world transform is stale"
                );

                let mut fresh = SceneCompiler::new();
                let fresh = fresh.compile(&source, &source.dirty_snapshot()).unwrap();
                let mut fresh_bounds = Vec::new();
                assert_eq!(
                    incremental_picture,
                    canonical(fresh, fresh.root_picture, &mut fresh_bounds),
                    "seed {seed}, step {step}"
                );
                for (incremental, fresh) in incremental_bounds.iter().zip(&fresh_bounds) {
                    assert_eq!(
                        incremental.union(*fresh),
                        *incremental,
                        "seed {seed}, step {step}: a span no longer covers its items"
                    );
                }
                source.acknowledge(&snapshot);
            }
        }
        assert!(
            COPIED_ITEMS.with(|copied| copied.get()) > 0,
            "no compile copied a clean subtree, so nothing above tested reuse"
        );
    }
}
