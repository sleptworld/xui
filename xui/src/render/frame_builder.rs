use std::collections::HashMap;

use super::{
    BuiltClipChain, BuiltClipChainId, BuiltDraw, BuiltDrawData, BuiltFrame, BuiltImage, BuiltItem,
    BuiltLayer, BuiltLayerId, BuiltLayerInstance, BuiltPath, BuiltShape, BuiltText, CompiledClipId,
    CompiledPictureItem, CompiledScene, ContentVersion, FrameProperties, LayerCacheId, PictureId,
    PlacementVersion, Primitive, RenderNodeId, SpatialNodeId,
};
use xui_interface::{Affine, Rect};

#[derive(Debug, Clone, PartialEq)]
pub enum FrameBuildError {
    MissingPicture(PictureId),
    MissingPrimitive(super::PrimitiveId),
    MissingSpatialNode(SpatialNodeId),
    MissingClip(CompiledClipId),
    DynamicTransformOnNonSpatialNode(RenderNodeId),
    DynamicCompositeOnNonIsolatedLayer(RenderNodeId),
}

impl std::fmt::Display for FrameBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingPicture(id) => write!(f, "compiled picture {id:?} is missing"),
            Self::MissingPrimitive(id) => write!(f, "compiled primitive {id:?} is missing"),
            Self::MissingSpatialNode(id) => write!(f, "compiled spatial node {id:?} is missing"),
            Self::MissingClip(id) => write!(f, "compiled clip {id:?} is missing"),
            Self::DynamicTransformOnNonSpatialNode(source) => write!(
                f,
                "dynamic transform target {source:?} is not a compiled spatial node"
            ),
            Self::DynamicCompositeOnNonIsolatedLayer(source) => write!(
                f,
                "dynamic composite target {source:?} is not an isolated picture"
            ),
        }
    }
}

impl std::error::Error for FrameBuildError {}

#[derive(Debug, Default)]
pub struct FrameBuilder {
    builds: u64,
}

impl FrameBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn build(
        &mut self,
        scene: &CompiledScene,
        viewport: Rect,
        properties: &FrameProperties,
    ) -> Result<BuiltFrame, FrameBuildError> {
        for source in properties.transform_sources() {
            if scene.contains_source(source) && scene.spatial_for_source(source).is_none() {
                return Err(FrameBuildError::DynamicTransformOnNonSpatialNode(source));
            }
        }
        for source in properties.composite_sources() {
            if scene.contains_source(source) && scene.layer_isolation(source) != Some(true) {
                return Err(FrameBuildError::DynamicCompositeOnNonIsolatedLayer(source));
            }
        }

        self.builds = self.builds.wrapping_add(1).max(1);
        let root_picture = scene
            .picture(scene.root_picture())
            .ok_or(FrameBuildError::MissingPicture(scene.root_picture()))?;
        let root_layer = BuiltLayerId(0);
        let frame = BuiltFrame {
            root_layer,
            layers: vec![BuiltLayer {
                source: root_picture.source,
                content_bounds: Rect::ZERO,
                render_bounds: viewport,
                content_version: ContentVersion::default(),
                cache_id: None,
                cache_policy: super::CachePolicy::None,
                items: Vec::new(),
                effects: std::sync::Arc::from([]),
            }],
            clip_chains: Vec::new(),
            live_layer_caches: Vec::new(),
            scene_revision: scene.scene_revision(),
            properties_revision: properties.revision(),
        };
        let mut context = BuildContext {
            scene,
            properties,
            viewport,
            frame,
            spatial_cache: HashMap::new(),
            clip_cache: HashMap::new(),
        };
        let result = context.build_picture_contents(scene.root_picture(), root_layer, true)?;
        let root = &mut context.frame.layers[root_layer.0];
        root.content_bounds = result.world_bounds.unwrap_or(Rect::ZERO);
        root.content_version = result.content_version;
        Ok(context.frame)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct BuildResult {
    world_bounds: Option<Rect>,
    content_version: ContentVersion,
}

#[derive(Debug, Clone, Copy)]
struct SpatialState {
    transform: Affine,
    content_version: ContentVersion,
}

#[derive(Debug, Clone, Copy)]
struct ClipState {
    id: BuiltClipChainId,
    bounds: Rect,
    content_version: ContentVersion,
}

struct BuildContext<'a> {
    scene: &'a CompiledScene,
    properties: &'a FrameProperties,
    viewport: Rect,
    frame: BuiltFrame,
    spatial_cache: HashMap<SpatialNodeId, SpatialState>,
    clip_cache: HashMap<CompiledClipId, ClipState>,
}

impl BuildContext<'_> {
    fn build_picture_contents(
        &mut self,
        picture_id: PictureId,
        layer_id: BuiltLayerId,
        cull_to_viewport: bool,
    ) -> Result<BuildResult, FrameBuildError> {
        let picture = self
            .scene
            .picture(picture_id)
            .ok_or(FrameBuildError::MissingPicture(picture_id))?;
        let mut result = BuildResult {
            content_version: picture.content_version,
            ..BuildResult::default()
        };
        for item in &picture.items {
            let item_result = match item {
                CompiledPictureItem::Primitive(id) => {
                    self.build_primitive(*id, layer_id, cull_to_viewport)?
                }
                CompiledPictureItem::Picture(id) => self.build_child_picture(*id, layer_id)?,
            };
            result = merge_results(result, item_result);
        }
        Ok(result)
    }

    fn build_primitive(
        &mut self,
        primitive_id: super::PrimitiveId,
        layer_id: BuiltLayerId,
        cull_to_viewport: bool,
    ) -> Result<BuildResult, FrameBuildError> {
        let primitive = self
            .scene
            .primitive(primitive_id)
            .ok_or(FrameBuildError::MissingPrimitive(primitive_id))?;
        let spatial = self.resolve_spatial(primitive.spatial)?;
        let local_bounds = primitive.primitive.paint_bounds();
        let world_bounds = spatial.transform.transform_rect(local_bounds);
        let clip = primitive.clip.map(|id| self.resolve_clip(id)).transpose()?;
        let visible_bounds = match clip {
            Some(clip) => intersect_rect(world_bounds, clip.bounds),
            None => Some(world_bounds),
        };
        let mut content_version = primitive.content_version.merge(spatial.content_version);
        if let Some(clip) = clip {
            content_version = content_version.merge(clip.content_version);
        }
        if visible_bounds
            .is_some_and(|bounds| !cull_to_viewport || bounds.intersects(self.viewport))
        {
            let common = BuiltDrawData {
                source: primitive.source,
                content_version,
                world_transform: spatial.transform,
                world_bounds,
                clip_chain: clip.map(|clip| clip.id),
            };
            let draw = match &primitive.primitive {
                Primitive::Shape(primitive) => BuiltDraw::Shape(BuiltShape {
                    common,
                    primitive: *primitive,
                }),
                Primitive::Path(primitive) => BuiltDraw::Path(BuiltPath {
                    common,
                    primitive: primitive.clone(),
                }),
                Primitive::Image(primitive) => BuiltDraw::Image(BuiltImage {
                    common,
                    primitive: primitive.clone(),
                }),
                Primitive::Text(primitive) => BuiltDraw::Text(BuiltText {
                    common,
                    primitive: *primitive,
                }),
            };
            self.frame.layers[layer_id.0]
                .items
                .push(BuiltItem::Draw(draw));
        }
        Ok(BuildResult {
            world_bounds: visible_bounds,
            content_version,
        })
    }

    fn build_child_picture(
        &mut self,
        picture_id: PictureId,
        parent_layer: BuiltLayerId,
    ) -> Result<BuildResult, FrameBuildError> {
        let picture = self
            .scene
            .picture(picture_id)
            .ok_or(FrameBuildError::MissingPicture(picture_id))?;
        let cache_id = Some(match picture.descriptor.cache_key {
            Some(key) => LayerCacheId::Explicit(key),
            None => LayerCacheId::Scene(picture.source),
        });
        let layer_id = self.frame.next_layer_id();
        self.frame.layers.push(BuiltLayer {
            source: picture.source,
            content_bounds: Rect::ZERO,
            render_bounds: Rect::ZERO,
            content_version: ContentVersion::default(),
            cache_id,
            cache_policy: picture.descriptor.cache_policy,
            items: Vec::new(),
            effects: picture.descriptor.effects.clone(),
        });
        if picture.descriptor.cache_policy != super::CachePolicy::None {
            self.frame
                .live_layer_caches
                .push(cache_id.expect("isolated picture has a cache identity"));
        }

        let child = self.build_picture_contents(picture_id, layer_id, false)?;
        let placement_spatial = self.resolve_spatial(picture.placement_spatial)?;
        let descriptor_bounds = picture
            .descriptor
            .bounds
            .map(|bounds| placement_spatial.transform.transform_rect(bounds));
        let content_bounds = descriptor_bounds
            .or(child.world_bounds)
            .unwrap_or(Rect::ZERO);
        let render_bounds = content_bounds.expand(picture.descriptor.effect_expansion());
        let mut composite = picture.descriptor.composite;
        let mut placement_version = PlacementVersion {
            scene: picture.composite_version,
            dynamic: 0,
        };
        if let Some(dynamic) = self.properties.composite(picture.source) {
            if let Some(opacity) = dynamic.value.opacity {
                composite.opacity = opacity;
            }
            if let Some(transform) = dynamic.value.transform {
                composite.transform = transform;
            }
            placement_version.dynamic = dynamic.revision;
        }
        let transformed_bounds = composite.transform.transform_rect(render_bounds);
        let placement_clip = picture
            .placement_clip
            .map(|id| self.resolve_clip(id))
            .transpose()?;
        let placement_bounds = match placement_clip {
            Some(clip) => intersect_rect(transformed_bounds, clip.bounds),
            None => Some(transformed_bounds),
        };
        let content_version = picture
            .content_version
            .merge(child.content_version)
            .merge(placement_spatial.content_version);
        let layer = &mut self.frame.layers[layer_id.0];
        layer.content_bounds = content_bounds;
        layer.render_bounds = render_bounds;
        layer.content_version = content_version;

        if placement_bounds.is_some_and(|bounds| bounds.intersects(self.viewport)) {
            self.frame.layers[parent_layer.0]
                .items
                .push(BuiltItem::Layer(BuiltLayerInstance {
                    source: picture.source,
                    layer: layer_id,
                    composite,
                    backdrop_effects: picture.descriptor.backdrop_effects.clone(),
                    clip_chain: placement_clip.map(|clip| clip.id),
                    world_bounds: placement_bounds.unwrap_or(Rect::ZERO),
                    placement_version,
                }));
        }
        let mut parent_version = content_version;
        parent_version.paint = parent_version.paint.max(placement_version.scene);
        parent_version.dynamic = parent_version.dynamic.max(placement_version.dynamic);
        Ok(BuildResult {
            world_bounds: placement_bounds,
            content_version: parent_version,
        })
    }

    fn resolve_spatial(&mut self, id: SpatialNodeId) -> Result<SpatialState, FrameBuildError> {
        if let Some(state) = self.spatial_cache.get(&id).copied() {
            return Ok(state);
        }
        let node = self
            .scene
            .spatial_node(id)
            .ok_or(FrameBuildError::MissingSpatialNode(id))?;
        let parent = node
            .parent
            .map(|parent| self.resolve_spatial(parent))
            .transpose()?
            .unwrap_or(SpatialState {
                transform: Affine::IDENTITY,
                content_version: ContentVersion::default(),
            });
        let dynamic = self.properties.transform(node.source);
        let local_transform = dynamic
            .map(|value| value.value)
            .unwrap_or(node.local_transform);
        let mut content_version = parent.content_version.merge(node.content_version);
        if let Some(dynamic) = dynamic {
            content_version.dynamic = content_version.dynamic.max(dynamic.revision);
        }
        let state = SpatialState {
            transform: local_transform.then(parent.transform),
            content_version,
        };
        self.spatial_cache.insert(id, state);
        Ok(state)
    }

    fn resolve_clip(&mut self, id: CompiledClipId) -> Result<ClipState, FrameBuildError> {
        if let Some(state) = self.clip_cache.get(&id).copied() {
            return Ok(state);
        }
        let clip = self
            .scene
            .clip(id)
            .ok_or(FrameBuildError::MissingClip(id))?
            .clone();
        let parent = clip
            .parent
            .map(|parent| self.resolve_clip(parent))
            .transpose()?;
        let spatial = self.resolve_spatial(clip.spatial)?;
        let world_bounds = spatial.transform.transform_rect(clip.clip.local_bounds());
        let effective_bounds = match parent {
            Some(parent) => intersect_rect(parent.bounds, world_bounds).unwrap_or(Rect::ZERO),
            None => world_bounds,
        };
        let mut content_version = clip.content_version.merge(spatial.content_version);
        if let Some(parent) = parent {
            content_version = content_version.merge(parent.content_version);
        }
        let built_id = BuiltClipChainId(self.frame.clip_chains.len());
        self.frame.clip_chains.push(BuiltClipChain {
            source: clip.source,
            parent: parent.map(|parent| parent.id),
            clip: clip.clip,
            world_transform: spatial.transform,
            world_bounds: effective_bounds,
        });
        let state = ClipState {
            id: built_id,
            bounds: effective_bounds,
            content_version,
        };
        self.clip_cache.insert(id, state);
        Ok(state)
    }
}

fn merge_results(a: BuildResult, b: BuildResult) -> BuildResult {
    BuildResult {
        world_bounds: match (a.world_bounds, b.world_bounds) {
            (Some(a), Some(b)) => Some(a.union(b)),
            (Some(bounds), None) | (None, Some(bounds)) => Some(bounds),
            (None, None) => None,
        },
        content_version: a.content_version.merge(b.content_version),
    }
}

pub(crate) fn intersect_rect(a: Rect, b: Rect) -> Option<Rect> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    (right > x && bottom > y).then(|| Rect::new(x, y, right - x, bottom - y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{ClipShape, LayerDescriptor, SceneCompiler, Shape, ShapePrimitive};
    use xui_interface::{Color, ComputedColorStyle, Point};

    fn shape(rect: Rect, color: Color) -> Primitive {
        Primitive::Shape(ShapePrimitive {
            bounds: rect,
            shape: Shape::Rect,
            fill: Some(ComputedColorStyle::Solid(color)),
            stroke: None,
            shadow: None,
        })
    }

    fn compile(source: &super::super::RenderScene) -> CompiledScene {
        let mut compiler = SceneCompiler::new();
        compiler
            .compile(source, &source.dirty_snapshot())
            .unwrap()
            .clone()
    }

    #[test]
    fn transform_clip_and_viewport_culling_are_resolved_per_frame() {
        let mut source = super::super::RenderScene::new();
        let transform = source.insert_transform(Affine::translate(10.0, 20.0));
        let clip = source.insert_clip(ClipShape::Rect(Rect::new(0.0, 0.0, 20.0, 20.0)));
        let visible = source.insert_primitive(shape(Rect::new(1.0, 1.0, 2.0, 2.0), Color::BLACK));
        source.append_child(source.root(), transform).unwrap();
        source.set_child(transform, Some(clip)).unwrap();
        source.set_child(clip, Some(visible)).unwrap();
        let outside =
            source.insert_primitive(shape(Rect::new(500.0, 500.0, 10.0, 10.0), Color::WHITE));
        source.append_child(source.root(), outside).unwrap();
        let outside_clip = source.insert_clip(ClipShape::Rect(Rect::new(500.0, 500.0, 10.0, 10.0)));
        let clipped_outside =
            source.insert_primitive(shape(Rect::new(0.0, 0.0, 1_000.0, 1_000.0), Color::WHITE));
        source.append_child(source.root(), outside_clip).unwrap();
        source
            .set_child(outside_clip, Some(clipped_outside))
            .unwrap();

        let scene = compile(&source);
        let frame = FrameBuilder::new()
            .build(
                &scene,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                &FrameProperties::default(),
            )
            .unwrap();
        assert_eq!(frame.clip_chains.len(), 2);
        assert_eq!(frame.layers[0].items.len(), 1);
        let BuiltItem::Draw(draw) = &frame.layers[0].items[0] else {
            panic!()
        };
        assert_eq!(
            draw.common()
                .world_transform
                .transform_point(Point::new(1.0, 1.0)),
            Point::new(11.0, 21.0)
        );
    }

    #[test]
    fn dynamic_transform_changes_frame_without_recompiling_scene() {
        let mut source = super::super::RenderScene::new();
        let transform = source.insert_transform(Affine::IDENTITY);
        let primitive =
            source.insert_primitive(shape(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLACK));
        source.append_child(source.root(), transform).unwrap();
        source.set_child(transform, Some(primitive)).unwrap();
        let scene = compile(&source);
        let scene_revision = scene.scene_revision();
        let mut properties = FrameProperties::default();
        properties.set_transform(transform, Affine::translate(30.0, 0.0));
        let mut builder = FrameBuilder::new();
        let first = builder
            .build(&scene, Rect::new(0.0, 0.0, 100.0, 100.0), &properties)
            .unwrap();
        let BuiltItem::Draw(draw) = &first.layers[0].items[0] else {
            panic!()
        };
        assert_eq!(draw.common().world_bounds.x, 30.0);
        let first_dynamic_version = draw.common().content_version.dynamic;

        properties.set_transform(transform, Affine::translate(40.0, 0.0));
        let second = builder
            .build(&scene, Rect::new(0.0, 0.0, 100.0, 100.0), &properties)
            .unwrap();
        let BuiltItem::Draw(draw) = &second.layers[0].items[0] else {
            panic!()
        };
        assert_eq!(draw.common().world_bounds.x, 40.0);
        assert!(draw.common().content_version.dynamic > first_dynamic_version);
        assert_eq!(scene.scene_revision(), scene_revision);
        assert_eq!(second.properties_revision, properties.revision());
    }

    #[test]
    fn dynamic_composite_changes_only_isolated_layer_placement_version() {
        let mut source = super::super::RenderScene::new();
        let layer = source.insert_layer(LayerDescriptor {
            force_offscreen: true,
            ..LayerDescriptor::default()
        });
        let primitive =
            source.insert_primitive(shape(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLACK));
        source.append_child(source.root(), layer).unwrap();
        source.set_child(layer, Some(primitive)).unwrap();
        let scene = compile(&source);
        let mut properties = FrameProperties::default();
        let mut builder = FrameBuilder::new();
        let first = builder
            .build(&scene, Rect::new(0.0, 0.0, 100.0, 100.0), &properties)
            .unwrap();
        let BuiltItem::Layer(first_instance) = &first.layers[0].items[0] else {
            panic!()
        };
        let first_layer_version = first.layers[first_instance.layer.0].content_version;

        properties.set_composite(
            layer,
            super::super::DynamicComposite {
                opacity: Some(0.5),
                transform: Some(Affine::translate(5.0, 0.0)),
            },
        );
        let second = builder
            .build(&scene, Rect::new(0.0, 0.0, 100.0, 100.0), &properties)
            .unwrap();
        let BuiltItem::Layer(second_instance) = &second.layers[0].items[0] else {
            panic!()
        };
        assert_eq!(
            second.layers[second_instance.layer.0].content_version,
            first_layer_version
        );
        assert_eq!(
            second_instance.placement_version.scene,
            first_instance.placement_version.scene
        );
        assert!(
            second_instance.placement_version.dynamic > first_instance.placement_version.dynamic
        );
        assert_eq!(second_instance.composite.opacity, 0.5);
        assert_eq!(second_instance.world_bounds.x, 5.0);
    }

    #[test]
    fn dynamic_composite_rejects_non_isolated_layer() {
        let mut source = super::super::RenderScene::new();
        let layer = source.insert_layer(LayerDescriptor::default());
        source.append_child(source.root(), layer).unwrap();
        let scene = compile(&source);
        let mut properties = FrameProperties::default();
        properties.set_composite(
            layer,
            super::super::DynamicComposite {
                opacity: Some(0.5),
                transform: None,
            },
        );

        assert_eq!(
            FrameBuilder::new()
                .build(&scene, Rect::new(0.0, 0.0, 100.0, 100.0), &properties,)
                .unwrap_err(),
            FrameBuildError::DynamicCompositeOnNonIsolatedLayer(layer)
        );
    }

    #[test]
    fn dynamic_transform_rejects_non_spatial_source() {
        let mut source = super::super::RenderScene::new();
        let primitive =
            source.insert_primitive(shape(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLACK));
        source.append_child(source.root(), primitive).unwrap();
        let scene = compile(&source);
        let mut properties = FrameProperties::default();
        properties.set_transform(primitive, Affine::translate(5.0, 0.0));

        assert_eq!(
            FrameBuilder::new()
                .build(&scene, Rect::new(0.0, 0.0, 100.0, 100.0), &properties,)
                .unwrap_err(),
            FrameBuildError::DynamicTransformOnNonSpatialNode(primitive)
        );
    }

    #[test]
    fn parent_clip_applies_to_picture_placement_not_cached_content() {
        let mut source = super::super::RenderScene::new();
        let clip = source.insert_clip(ClipShape::Rect(Rect::new(0.0, 0.0, 25.0, 25.0)));
        let layer = source.insert_layer(LayerDescriptor {
            force_offscreen: true,
            ..LayerDescriptor::default()
        });
        let primitive =
            source.insert_primitive(shape(Rect::new(0.0, 0.0, 50.0, 50.0), Color::BLACK));
        source.append_child(source.root(), clip).unwrap();
        source.set_child(clip, Some(layer)).unwrap();
        source.set_child(layer, Some(primitive)).unwrap();
        let scene = compile(&source);
        let frame = FrameBuilder::new()
            .build(
                &scene,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                &FrameProperties::default(),
            )
            .unwrap();
        let BuiltItem::Layer(instance) = &frame.layers[0].items[0] else {
            panic!()
        };
        assert!(instance.clip_chain.is_some());
        let BuiltItem::Draw(draw) = &frame.layers[instance.layer.0].items[0] else {
            panic!()
        };
        assert_eq!(draw.common().clip_chain, None);
        assert_eq!(instance.world_bounds, Rect::new(0.0, 0.0, 25.0, 25.0));
    }
}
