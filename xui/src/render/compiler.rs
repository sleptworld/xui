use std::{collections::HashSet, sync::Arc};

use slotmap::{SecondaryMap, SlotMap};

use super::{
    CompiledClip, CompiledClipId, CompiledItemSpan, CompiledPicture, CompiledPictureItem,
    CompiledPrimitive, CompiledScene, CompiledSpatialNode, ContentVersion, DirtySnapshot,
    LayerCacheKey, LayerDescriptor, PictureId, PrimitiveId, RenderDirty, RenderNodeId,
    RenderNodeKind, RenderScene, SceneError, SpatialNodeId, render_graph::BuiltLayerProgram,
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
        if needs_initial
            || changed
                .iter()
                .any(|(_, dirty)| dirty.intersects(RenderDirty::TOPOLOGY | RenderDirty::EFFECT))
        {
            validate_explicit_cache_keys(source)?;
        }

        let structural = needs_initial
            || changed.iter().any(|(id, dirty)| {
                requires_structural_rebuild(source, self.compiled.as_ref(), *id, *dirty)
            });

        if structural {
            // Taken rather than cloned: a deep copy of every SlotMap and
            // source index costs O(scene) on each structural frame.
            let mut next = match self.compiled.take() {
                Some(current) => current,
                None => empty_compiled_scene(source),
            };
            let outcome = if needs_initial {
                rebuild_structure(&mut next, source)
            } else {
                rebuild_structural_branches(&mut next, source, &changed)
            };
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
        dirty |= node.dirty & (RenderDirty::PAINT | RenderDirty::EFFECT);
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
        RenderNodeKind::Primitive(_) => !compiled.primitive_by_source.contains_key(&id),
        RenderNodeKind::Transform(_) => !compiled.spatial_by_source.contains_key(&id),
        RenderNodeKind::Clip(_) => !compiled.clip_by_source.contains_key(&id),
        RenderNodeKind::Layer(layer) => {
            layer.descriptor.requires_isolation() != compiled.picture_by_source.contains_key(&id)
        }
    }
}

/// Re-derives a primitive's paint bounds in `ancestor`'s local space by
/// composing the local transforms between them.
fn bounds_in_spatial(
    compiled: &CompiledScene,
    primitive: &CompiledPrimitive,
    ancestor: SpatialNodeId,
) -> Option<Bounds> {
    let mut transform = Affine::IDENTITY;
    let mut current = primitive.spatial;
    while current != ancestor {
        let node = compiled.spatial_nodes.get(current)?;
        transform = transform.then(node.local_transform);
        current = node.parent?;
    }
    Some(transform.transform_bounds(primitive.primitive.paint_bounds()))
}

/// Refreshes span bounds after a non-structural geometry update.
///
/// Such an update moves primitives and spatial nodes without changing the
/// picture's item order, so every span's range stays valid and only the unions
/// need redoing. Clips are deliberately not re-applied: ignoring them yields a
/// bound at least as large as the original, which can only cull less.
fn refresh_span_bounds(compiled: &mut CompiledScene) {
    let picture_ids: Vec<_> = compiled
        .pictures
        .iter()
        .filter(|(_, picture)| !picture.spans.is_empty())
        .map(|(id, _)| id)
        .collect();

    for picture_id in picture_ids {
        let mut spans = std::mem::take(&mut compiled.pictures[picture_id].spans);
        let items = std::mem::take(&mut compiled.pictures[picture_id].items);
        for span in &mut spans {
            let mut bounds: Option<Bounds> = None;
            for item in &items[span.start as usize..span.end as usize] {
                let CompiledPictureItem::Primitive(primitive_id) = item else {
                    continue;
                };
                let Some(primitive) = compiled.primitives.get(*primitive_id) else {
                    continue;
                };
                let Some(local) = bounds_in_spatial(compiled, primitive, span.spatial) else {
                    continue;
                };
                bounds = Some(match bounds {
                    Some(bounds) => bounds.union(local),
                    None => local,
                });
            }
            if let Some(bounds) = bounds {
                span.local_bounds = bounds;
            }
        }
        compiled.pictures[picture_id].spans = spans;
        compiled.pictures[picture_id].items = items;
    }
}

fn apply_non_structural_updates(
    compiled: &mut CompiledScene,
    source: &RenderScene,
    changed: &[(RenderNodeId, RenderDirty)],
) -> Result<(), SceneCompileError> {
    let mut geometry_moved = false;
    for (id, dirty) in changed {
        let node = source.node(*id).ok_or(SceneError::MissingNode(*id))?;
        match &node.kind {
            RenderNodeKind::Group(_) => {}
            RenderNodeKind::Primitive(value) => {
                let primitive = compiled
                    .primitive_by_source
                    .get(id)
                    .and_then(|key| compiled.primitives.get_mut(*key))
                    .ok_or(SceneError::MissingNode(*id))?;
                if dirty.intersects(RenderDirty::GEOMETRY | RenderDirty::PAINT) {
                    primitive.primitive = value.primitive.clone();
                    primitive.content_version = node.epochs.content_version();
                    geometry_moved |= dirty.contains(RenderDirty::GEOMETRY);
                }
            }
            RenderNodeKind::Transform(value) => {
                let spatial = compiled
                    .spatial_by_source
                    .get(id)
                    .and_then(|key| compiled.spatial_nodes.get_mut(*key))
                    .ok_or(SceneError::MissingNode(*id))?;
                if dirty.contains(RenderDirty::GEOMETRY) {
                    spatial.local_transform = value.transform;
                    spatial.content_version = node.epochs.content_version();
                    geometry_moved = true;
                }
            }
            RenderNodeKind::Clip(value) => {
                let clip = compiled
                    .clip_by_source
                    .get(id)
                    .and_then(|key| compiled.clips.get_mut(*key))
                    .ok_or(SceneError::MissingNode(*id))?;
                if dirty.intersects(RenderDirty::CLIP | RenderDirty::GEOMETRY) {
                    clip.clip = value.clip.clone();
                    clip.content_version = node.epochs.content_version();
                }
            }
            RenderNodeKind::Layer(value) => {
                if let Some(picture_id) = compiled.picture_by_source.get(id).copied() {
                    let render_program =
                        reusable_render_program(compiled, picture_id, *id, &value.descriptor)?;
                    let picture = compiled
                        .pictures
                        .get_mut(picture_id)
                        .ok_or(SceneError::MissingNode(*id))?;
                    picture.descriptor = value.descriptor.clone();
                    picture.render_program = Some(render_program);
                    picture.content_version = node.epochs.content_version();
                    picture.composite_version = node.epochs.composite;
                }
            }
        }
    }
    // Span bounds are derived from primitive geometry and local transforms, so
    // a geometry-only update would otherwise leave them stale and let the
    // builder cull content that has moved back into view.
    if geometry_moved {
        refresh_span_bounds(compiled);
    }
    Ok(())
}

fn empty_compiled_scene(source: &RenderScene) -> CompiledScene {
    let mut pictures = SlotMap::with_key();
    let mut spatial_nodes = SlotMap::with_key();
    let root_spatial = spatial_nodes.insert(CompiledSpatialNode {
        source: source.root(),
        parent: None,
        local_transform: Affine::IDENTITY,
        content_version: ContentVersion::default(),
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
        is_root: true,
    });
    CompiledScene {
        root_picture,
        root_spatial,
        pictures,
        primitives: SlotMap::with_key(),
        spatial_nodes,
        clips: SlotMap::with_key(),
        picture_by_source: [(source.root(), root_picture)].into_iter().collect(),
        primitive_by_source: Default::default(),
        spatial_by_source: Default::default(),
        clip_by_source: Default::default(),
        source_epoch: Default::default(),
        layer_isolation: Default::default(),
        metadata_epoch: 0,
        scene_revision: 0,
    }
}

/// Mark set for the compiled-graph sweep. Every id here is a slotmap key, so
/// `SecondaryMap` indexes it directly instead of hashing it once per entity.
#[derive(Default)]
struct LiveCompiledIds {
    pictures: SecondaryMap<PictureId, ()>,
    primitives: SecondaryMap<PrimitiveId, ()>,
    spatials: SecondaryMap<SpatialNodeId, ()>,
    clips: SecondaryMap<CompiledClipId, ()>,
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
fn finalize_picture_spans(picture: &mut CompiledPicture) {
    picture
        .spans
        .retain(|span| span.end - span.start >= MIN_SPAN_ITEMS);
    picture
        .spans
        .sort_unstable_by_key(|span| (span.start, std::cmp::Reverse(span.end)));
}

#[derive(Clone, Copy)]
struct WalkContext {
    picture: PictureId,
    spatial: SpatialNodeId,
    clip: Option<CompiledClipId>,
}

fn rebuild_structure(
    compiled: &mut CompiledScene,
    source: &RenderScene,
) -> Result<(), SceneCompileError> {
    let mut live = LiveCompiledIds::default();
    live.pictures.insert(compiled.root_picture, ());
    live.spatials.insert(compiled.root_spatial, ());
    refresh_source_metadata(compiled, source)?;

    let root_node = source
        .node(source.root())
        .ok_or(SceneError::MissingNode(source.root()))?;
    *compiled
        .spatial_nodes
        .get_mut(compiled.root_spatial)
        .expect("root spatial is retained") = CompiledSpatialNode {
        source: source.root(),
        parent: None,
        local_transform: Affine::IDENTITY,
        content_version: ContentVersion::default(),
    };
    *compiled
        .pictures
        .get_mut(compiled.root_picture)
        .expect("root picture is retained") = CompiledPicture {
        source: source.root(),
        items: Vec::new(),
        spans: Vec::new(),
        descriptor: LayerDescriptor::default(),
        render_program: None,
        placement_spatial: compiled.root_spatial,
        placement_clip: None,
        content_version: root_node.epochs.content_version(),
        composite_version: root_node.epochs.composite,
        is_root: true,
    };
    compiled
        .picture_by_source
        .insert(source.root(), compiled.root_picture);

    walk_node(
        compiled,
        source,
        source.root(),
        WalkContext {
            picture: compiled.root_picture,
            spatial: compiled.root_spatial,
            clip: None,
        },
        &mut live,
    )?;
    finalize_picture_spans(&mut compiled.pictures[compiled.root_picture]);
    retain_live(compiled, &live);
    Ok(())
}

fn refresh_source_metadata(
    compiled: &mut CompiledScene,
    source: &RenderScene,
) -> Result<(), SceneCompileError> {
    // Bumping the epoch retires every previous stamp, so neither map has to be
    // cleared (and regrown) on each structural frame.
    compiled.metadata_epoch = compiled.metadata_epoch.wrapping_add(1);
    let epoch = compiled.metadata_epoch;
    for (id, node) in source.depth_first(source.root())? {
        compiled.source_epoch.insert(id, epoch);
        if let RenderNodeKind::Layer(layer) = &node.kind {
            compiled
                .layer_isolation
                .insert(id, (epoch, layer.descriptor.requires_isolation()));
        }
    }
    Ok(())
}

fn rebuild_structural_branches(
    compiled: &mut CompiledScene,
    source: &RenderScene,
    changed: &[(RenderNodeId, RenderDirty)],
) -> Result<(), SceneCompileError> {
    let mut branch_sources = Vec::new();
    for (id, dirty) in changed {
        let branch = structural_branch_source(compiled, source, *id, *dirty);

        // ROOT
        let Some(branch) = branch else {
            return rebuild_structure(compiled, source);
        };

        if branch_sources
            .iter()
            .any(|existing| is_source_ancestor(source, *existing, branch))
        {
            continue;
        }
        branch_sources.retain(|existing| !is_source_ancestor(source, branch, *existing));
        branch_sources.push(branch);
    }

    refresh_source_metadata(compiled, source)?;
    for source_id in branch_sources {
        rebuild_picture_contents(compiled, source, source_id)?;
    }
    retain_picture_graph(compiled);
    Ok(())
}

fn structural_branch_source(
    compiled: &CompiledScene,
    source: &RenderScene,
    id: RenderNodeId,
    dirty: RenderDirty,
) -> Option<RenderNodeId> {
    let Some(node) = source.node(id) else {
        return None;
    };
    let can_rebuild_self = !dirty.contains(RenderDirty::VISIBILITY)
        && dirty.intersects(RenderDirty::TOPOLOGY | RenderDirty::EFFECT)
        && matches!(
            &node.kind,
            RenderNodeKind::Layer(layer)
                if layer.descriptor.requires_isolation()
                    && compiled.picture_by_source.contains_key(&id)
        );
    let mut current = if can_rebuild_self {
        Some(id)
    } else {
        node.parent
    };
    while let Some(candidate) = current {
        // The root picture is synthetic and its source is the root Group, not an
        // isolated Layer. Returning it as a branch would send it to
        // `rebuild_picture_contents`, which only accepts isolated layers.
        // Falling back to `rebuild_structure` also refreshes the retained root
        // spatial and picture state.
        if candidate == source.root() {
            return None;
        }
        if compiled.picture_by_source.contains_key(&candidate) {
            return Some(candidate);
        }
        current = source.node(candidate).and_then(|node| node.parent);
    }

    None
}

fn is_source_ancestor(
    source: &RenderScene,
    ancestor: RenderNodeId,
    descendant: RenderNodeId,
) -> bool {
    let mut current = Some(descendant);
    while let Some(id) = current {
        if id == ancestor {
            return true;
        }
        current = source.node(id).and_then(|node| node.parent);
    }
    false
}

fn rebuild_picture_contents(
    compiled: &mut CompiledScene,
    source: &RenderScene,
    source_id: RenderNodeId,
) -> Result<(), SceneCompileError> {
    let picture_id = compiled
        .picture_by_source
        .get(&source_id)
        .copied()
        .ok_or(SceneError::MissingNode(source_id))?;
    let node = source
        .node(source_id)
        .ok_or(SceneError::MissingNode(source_id))?;
    let RenderNodeKind::Layer(layer) = &node.kind else {
        return Err(SceneError::WrongNodeKind {
            node: source_id,
            expected: "isolated layer",
        }
        .into());
    };
    let render_program =
        reusable_render_program(compiled, picture_id, source_id, &layer.descriptor)?;
    let picture = compiled
        .pictures
        .get_mut(picture_id)
        .ok_or(SceneError::MissingNode(source_id))?;
    picture.items.clear();
    picture.spans.clear();
    picture.descriptor = layer.descriptor.clone();
    picture.render_program = Some(render_program);
    picture.content_version = node.epochs.content_version();
    picture.composite_version = node.epochs.composite;
    let context = WalkContext {
        picture: picture_id,
        spatial: picture.placement_spatial,
        clip: None,
    };
    let mut branch_live = LiveCompiledIds::default();
    branch_live.pictures.insert(picture_id, ());
    if let Some(child) = layer.child {
        walk_node(compiled, source, child, context, &mut branch_live)?;
    }
    finalize_picture_spans(&mut compiled.pictures[picture_id]);
    Ok(())
}

fn walk_node(
    compiled: &mut CompiledScene,
    source: &RenderScene,
    id: RenderNodeId,
    context: WalkContext,
    live: &mut LiveCompiledIds,
) -> Result<WalkOutput, SceneCompileError> {
    let node = source.node(id).ok_or(SceneError::MissingNode(id))?;
    if !node.visible {
        return Ok(WalkOutput::EMPTY);
    }

    let output = match &node.kind {
        RenderNodeKind::Group(group) => {
            let mut output = WalkOutput::EMPTY;
            for child in &group.children {
                output = output.merge(walk_node(compiled, source, *child, context, live)?);
            }
            output
        }
        RenderNodeKind::Primitive(value) => {
            let primitive_id = match compiled.primitive_by_source.get(&id).copied() {
                Some(key) if compiled.primitives.contains_key(key) => key,
                _ => {
                    let key = compiled.primitives.insert(CompiledPrimitive {
                        source: id,
                        primitive: value.primitive.clone(),
                        spatial: context.spatial,
                        clip: context.clip,
                        content_version: node.epochs.content_version(),
                    });
                    compiled.primitive_by_source.insert(id, key);
                    key
                }
            };
            *compiled
                .primitives
                .get_mut(primitive_id)
                .expect("primitive was retained") = CompiledPrimitive {
                source: id,
                primitive: value.primitive.clone(),
                spatial: context.spatial,
                clip: context.clip,
                content_version: node.epochs.content_version(),
            };
            live.primitives.insert(primitive_id, ());
            compiled.pictures[context.picture]
                .items
                .push(CompiledPictureItem::Primitive(primitive_id));
            WalkOutput {
                local_bounds: Some(value.primitive.paint_bounds()),
                boundable: true,
            }
        }
        RenderNodeKind::Transform(value) => {
            let spatial_id = match compiled.spatial_by_source.get(&id).copied() {
                Some(key) if compiled.spatial_nodes.contains_key(key) => key,
                _ => {
                    let key = compiled.spatial_nodes.insert(CompiledSpatialNode {
                        source: id,
                        parent: Some(context.spatial),
                        local_transform: value.transform,
                        content_version: node.epochs.content_version(),
                    });
                    compiled.spatial_by_source.insert(id, key);
                    key
                }
            };
            *compiled
                .spatial_nodes
                .get_mut(spatial_id)
                .expect("spatial node was retained") = CompiledSpatialNode {
                source: id,
                parent: Some(context.spatial),
                local_transform: value.transform,
                content_version: node.epochs.content_version(),
            };
            live.spatials.insert(spatial_id, ());
            let Some(child) = value.child else {
                return Ok(WalkOutput::EMPTY);
            };
            let start = compiled.pictures[context.picture].items.len() as u32;
            let inner = walk_node(
                compiled,
                source,
                child,
                WalkContext {
                    spatial: spatial_id,
                    ..context
                },
                live,
            )?;
            let end = compiled.pictures[context.picture].items.len() as u32;
            // `inner.local_bounds` is already in `spatial_id`'s local space,
            // which is exactly what the span records.
            if let Some(local_bounds) = inner.local_bounds.filter(|_| inner.boundable) {
                compiled.pictures[context.picture]
                    .spans
                    .push(CompiledItemSpan {
                        spatial: spatial_id,
                        local_bounds,
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
            let clip_id = match compiled.clip_by_source.get(&id).copied() {
                Some(key) if compiled.clips.contains_key(key) => key,
                _ => {
                    let key = compiled.clips.insert(CompiledClip {
                        source: id,
                        parent: context.clip,
                        spatial: context.spatial,
                        clip: value.clip.clone(),
                        content_version: node.epochs.content_version(),
                    });
                    compiled.clip_by_source.insert(id, key);
                    key
                }
            };
            *compiled.clips.get_mut(clip_id).expect("clip was retained") = CompiledClip {
                source: id,
                parent: context.clip,
                spatial: context.spatial,
                clip: value.clip.clone(),
                content_version: node.epochs.content_version(),
            };
            live.clips.insert(clip_id, ());
            let Some(child) = value.child else {
                return Ok(WalkOutput::EMPTY);
            };
            let inner = walk_node(
                compiled,
                source,
                child,
                WalkContext {
                    clip: Some(clip_id),
                    ..context
                },
                live,
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
            compiled.picture_by_source.remove(&id);
            match value.child {
                Some(child) => walk_node(compiled, source, child, context, live)?,
                None => WalkOutput::EMPTY,
            }
        }
        RenderNodeKind::Layer(value) => {
            let render_program = match compiled.picture_by_source.get(&id).copied() {
                Some(picture_id) => {
                    reusable_render_program(compiled, picture_id, id, &value.descriptor)?
                }
                None => compile_render_program(id, &value.descriptor)?,
            };
            let picture_id = match compiled.picture_by_source.get(&id).copied() {
                Some(key) if compiled.pictures.contains_key(key) => key,
                _ => {
                    let key = compiled.pictures.insert(CompiledPicture {
                        source: id,
                        items: Vec::new(),
                        spans: Vec::new(),
                        descriptor: value.descriptor.clone(),
                        render_program: Some(render_program.clone()),
                        placement_spatial: context.spatial,
                        placement_clip: context.clip,
                        content_version: node.epochs.content_version(),
                        composite_version: node.epochs.composite,
                        is_root: false,
                    });
                    compiled.picture_by_source.insert(id, key);
                    key
                }
            };
            *compiled
                .pictures
                .get_mut(picture_id)
                .expect("picture was retained") = CompiledPicture {
                source: id,
                items: Vec::new(),
                spans: Vec::new(),
                descriptor: value.descriptor.clone(),
                render_program: Some(render_program),
                placement_spatial: context.spatial,
                placement_clip: context.clip,
                content_version: node.epochs.content_version(),
                composite_version: node.epochs.composite,
                is_root: false,
            };
            live.pictures.insert(picture_id, ());
            compiled.pictures[context.picture]
                .items
                .push(CompiledPictureItem::Picture(picture_id));
            if let Some(child) = value.child {
                walk_node(
                    compiled,
                    source,
                    child,
                    WalkContext {
                        picture: picture_id,
                        clip: None,
                        ..context
                    },
                    live,
                )?;
            }
            finalize_picture_spans(&mut compiled.pictures[picture_id]);
            // An isolated layer's effects can paint outside its content, so
            // enclosing spans must not try to bound it.
            WalkOutput {
                local_bounds: None,
                boundable: false,
            }
        }
    };
    Ok(output)
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

fn retain_live(compiled: &mut CompiledScene, live: &LiveCompiledIds) {
    // let stale_pictures: Vec<_> = compiled
    //     .pictures
    //     .keys()
    //     .filter(|key| !live.pictures.contains(key))
    //     .collect();
    // for key in stale_pictures {
    //     compiled.pictures.remove(key);
    // }
    compiled
        .pictures
        .retain(|k, _| live.pictures.contains_key(k));
    compiled
        .primitives
        .retain(|k, _| live.primitives.contains_key(k));
    compiled
        .spatial_nodes
        .retain(|k, _| live.spatials.contains_key(k));
    compiled.clips.retain(|k, _| live.clips.contains_key(k));
    // let stale_primitives: Vec<_> = compiled
    //     .primitives
    //     .keys()
    //     .filter(|key| !live.primitives.contains(key))
    //     .collect();
    // for key in stale_primitives {
    //     compiled.primitives.remove(key);
    // }
    // let stale_spatials: Vec<_> = compiled
    //     .spatial_nodes
    //     .keys()
    //     .filter(|key| !live.spatials.contains(key))
    //     .collect();
    // for key in stale_spatials {
    //     compiled.spatial_nodes.remove(key);
    // }
    // let stale_clips: Vec<_> = compiled
    //     .clips
    //     .keys()
    //     .filter(|key| !live.clips.contains(key))
    //     .collect();
    // for key in stale_clips {
    //     compiled.clips.remove(key);
    // }
    compiled
        .picture_by_source
        .retain(|_, key| live.pictures.contains_key(*key));
    compiled
        .primitive_by_source
        .retain(|_, key| live.primitives.contains_key(*key));
    compiled
        .spatial_by_source
        .retain(|_, key| live.spatials.contains_key(*key));
    compiled
        .clip_by_source
        .retain(|_, key| live.clips.contains_key(*key));
}

fn retain_picture_graph(compiled: &mut CompiledScene) {
    let mut live = LiveCompiledIds::default();
    let mut pictures = vec![compiled.root_picture];
    while let Some(picture_id) = pictures.pop() {
        if live.pictures.insert(picture_id, ()).is_some() {
            continue;
        }
        let Some(picture) = compiled.pictures.get(picture_id).cloned() else {
            continue;
        };
        mark_spatial_chain(compiled, &mut live, picture.placement_spatial);
        if let Some(clip) = picture.placement_clip {
            mark_clip_chain(compiled, &mut live, clip);
        }
        for item in picture.items {
            match item {
                CompiledPictureItem::Primitive(primitive_id) => {
                    if live.primitives.insert(primitive_id, ()).is_some() {
                        continue;
                    }
                    let Some(primitive) = compiled.primitives.get(primitive_id).cloned() else {
                        continue;
                    };
                    mark_spatial_chain(compiled, &mut live, primitive.spatial);
                    if let Some(clip) = primitive.clip {
                        mark_clip_chain(compiled, &mut live, clip);
                    }
                }
                CompiledPictureItem::Picture(child) => pictures.push(child),
            }
        }
    }
    retain_live(compiled, &live);
}

fn mark_spatial_chain(
    compiled: &CompiledScene,
    live: &mut LiveCompiledIds,
    mut spatial: SpatialNodeId,
) {
    loop {
        if live.spatials.insert(spatial, ()).is_some() {
            break;
        }
        let Some(parent) = compiled
            .spatial_nodes
            .get(spatial)
            .and_then(|node| node.parent)
        else {
            break;
        };
        spatial = parent;
    }
}

fn mark_clip_chain(
    compiled: &CompiledScene,
    live: &mut LiveCompiledIds,
    mut clip_id: CompiledClipId,
) {
    loop {
        if live.clips.insert(clip_id, ()).is_some() {
            break;
        }
        let Some(clip) = compiled.clips.get(clip_id) else {
            break;
        };
        mark_spatial_chain(compiled, live, clip.spatial);
        let Some(parent) = clip.parent else {
            break;
        };
        clip_id = parent;
    }
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
        BlendMode, CachePolicy, ClipShape, CompositeOperator, CompositeStyle, Primitive, Shape,
        ShapePrimitive,
    };
    use std::sync::Arc;
    use xui_interface::{
        Bounds, Color, ComputedColorStyle, ComputedEffect, FilterQuality, ImageData, ImageKey,
        Rect, Size,
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
        let first_primitive = first.primitive_by_source[&primitive];
        let first_spatial = first.spatial_by_source[&transform];

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

        assert_eq!(second.primitive_by_source[&primitive], first_primitive);
        assert_eq!(second.spatial_by_source[&transform], first_spatial);
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
        let first_id = compiler.compiled_scene().unwrap().primitive_by_source[&primitive];
        let second = compiler.compile(&source, &snapshot).unwrap();
        assert_eq!(second.scene_revision(), first_revision);
        assert_eq!(second.primitive_by_source[&primitive], first_id);
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
            .primitive_by_source[&first];

        source.remove_subtree(stale).unwrap();
        let snapshot = source.dirty_snapshot();
        let compiled = compiler.compile(&source, &snapshot).unwrap();
        assert_eq!(compiled.primitive_by_source[&first], first_id);
        assert!(!compiled.primitive_by_source.contains_key(&stale));
        assert_eq!(compiled.pictures[compiled.root_picture].items.len(), 1);
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
    fn topology_inside_an_isolated_layer_targets_that_picture_branch() {
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
        let compiled = compiler.compiled_scene().unwrap();
        assert_eq!(
            structural_branch_source(compiled, &source, group, RenderDirty::TOPOLOGY),
            Some(layer)
        );

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
}
