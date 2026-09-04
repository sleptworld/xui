//! Frame-level tests, currently disabled: they are written against the text
//! engine that is being replaced, and are kept here until that lands.

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use xui::render::{
//         BackdropIsolation, BuiltDrawData, BuiltLayer, BuiltLayerInstance, BuiltShape, CachePolicy,
//         CompositePrefix, CompositePrefixId, CompositeStyle, ContentVersion, LayerDescriptor,
//         PlacementVersion, ShapePrimitive, SurfacePrefix,
//     };
//     use xui_interface::{
//         ComputedBackdropFilter, ComputedBackdropMask, ComputedBackdropStyle, ComputedEffect,
//         FilterQuality, ImageKey, Point,
//     };
//     use xui_cosmic::CosmicEngine;

//     type TestBackend = super::SkiaBackend<CosmicEngine>;

//     fn shape_frame(clip: Option<Rect>) -> BuiltFrame {
//         let source = xui::render::RenderNodeId::default();
//         let clip_chains = clip
//             .map(|rect| {
//                 vec![xui::render::BuiltClipChain {
//                     source,
//                     parent: None,
//                     clip: ClipShape::Rect(rect),
//                     world_transform: Affine::IDENTITY,
//                     world_bounds: rect,
//                 }]
//             })
//             .unwrap_or_default();
//         BuiltFrame {
//             root_layer: BuiltLayerId(0),
//             layers: vec![BuiltLayer {
//                 source,
//                 content_bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
//                 render_bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
//                 content_version: ContentVersion::default(),
//                 cache_id: None,
//                 cache_policy: CachePolicy::None,
//                 backdrop_isolation: BackdropIsolation::Isolate,
//                 items: vec![BuiltItem::Draw(BuiltDraw::Shape(BuiltShape {
//                     common: BuiltDrawData {
//                         source,
//                         content_version: ContentVersion::default(),
//                         world_transform: Affine::IDENTITY,
//                         world_bounds: Rect::new(1.0, 1.0, 8.0, 8.0),
//                         clip_chain: clip.map(|_| BuiltClipChainId(0)),
//                     },
//                     primitive: ShapePrimitive {
//                         bounds: Rect::new(1.0, 1.0, 8.0, 8.0),
//                         shape: Shape::Rect,
//                         fill: Some(ComputedColorStyle::Solid(Color::rgb(1.0, 0.0, 0.0))),
//                         stroke: None,
//                         shadow: None,
//                     },
//                 }))],
//             }],
//             layer_instances: Vec::new(),
//             composite_prefixes: Vec::new(),
//             clip_chains,
//             live_layer_caches: Vec::new(),
//             scene_revision: 1,
//             properties_revision: 0,
//         }
//     }

//     fn pixel(pixels: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
//         let index = (y * width + x) * 4;
//         pixels[index..index + 4].try_into().unwrap()
//     }

//     fn layer_instance(descriptor: &LayerDescriptor) -> BuiltLayerInstance {
//         let source = xui::render::RenderNodeId::default();
//         BuiltLayerInstance {
//             source,
//             layer: BuiltLayerId(0),
//             composite: descriptor.composite.render_graph_instance(),
//             render_program: descriptor
//                 .bind_render_program(Arc::new(descriptor.compile_render_program().unwrap()))
//                 .unwrap(),
//             clip_chain: None,
//             world_bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
//             placement_version: PlacementVersion::default(),
//             destination_prefix: None,
//         }
//     }

//     fn shape_draw(rect: Rect, color: Color) -> BuiltItem {
//         let source = xui::render::RenderNodeId::default();
//         BuiltItem::Draw(BuiltDraw::Shape(BuiltShape {
//             common: BuiltDrawData {
//                 source,
//                 content_version: ContentVersion::default(),
//                 world_transform: Affine::IDENTITY,
//                 world_bounds: rect,
//                 clip_chain: None,
//             },
//             primitive: ShapePrimitive {
//                 bounds: rect,
//                 shape: Shape::Rect,
//                 fill: Some(ComputedColorStyle::Solid(color)),
//                 stroke: None,
//                 shadow: None,
//             },
//         }))
//     }

//     fn layered_frame(
//         descriptor: &LayerDescriptor,
//         mut root_items: Vec<BuiltItem>,
//         child_items: Vec<BuiltItem>,
//     ) -> BuiltFrame {
//         let source = xui::render::RenderNodeId::default();
//         let item_count = root_items.len();
//         let needs_backdrop = descriptor
//             .compile_render_program()
//             .unwrap()
//             .external_resource(ExternalResourceKind::Backdrop)
//             .is_some();
//         root_items.push(BuiltItem::Layer(xui::render::BuiltLayerInstanceId(0)));
//         let mut instance = layer_instance(descriptor);
//         instance.layer = BuiltLayerId(1);
//         instance.destination_prefix = needs_backdrop.then_some(CompositePrefixId(0));
//         BuiltFrame {
//             root_layer: BuiltLayerId(0),
//             layers: vec![
//                 BuiltLayer {
//                     source,
//                     content_bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
//                     render_bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
//                     content_version: ContentVersion::default(),
//                     cache_id: None,
//                     cache_policy: CachePolicy::None,
//                     backdrop_isolation: BackdropIsolation::Isolate,
//                     items: root_items,
//                 },
//                 BuiltLayer {
//                     source,
//                     content_bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
//                     render_bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
//                     content_version: ContentVersion::default(),
//                     cache_id: None,
//                     cache_policy: CachePolicy::None,
//                     backdrop_isolation: BackdropIsolation::Isolate,
//                     items: child_items,
//                 },
//             ],
//             layer_instances: vec![instance],
//             composite_prefixes: if needs_backdrop {
//                 vec![CompositePrefix {
//                     parent: None,
//                     local: SurfacePrefix {
//                         layer: BuiltLayerId(0),
//                         item_count,
//                     },
//                     placement: None,
//                 }]
//             } else {
//                 Vec::new()
//             },
//             clip_chains: Vec::new(),
//             live_layer_caches: Vec::new(),
//             scene_revision: 1,
//             properties_revision: 0,
//         }
//     }

//     fn render_frame(frame: &BuiltFrame) -> Vec<u8> {
//         let mut backend = TestBackend::headless(
//             1.0,
//             SkiaBackendOptions {
//                 clear_color: Color::BLACK,
//                 ..SkiaBackendOptions::default()
//             },
//         );
//         let mut text = TextHost::new(CosmicEngine::new(1.0));
//         <TestBackend as RenderBackend<TextHost<CosmicEngine>>>::begin_frame(
//             &mut backend,
//             Size::new(10.0, 10.0),
//         )
//         .unwrap();
//         backend.submit(frame, &mut text).unwrap();
//         <TestBackend as RenderBackend<TextHost<CosmicEngine>>>::end_frame(&mut backend).unwrap();
//         backend.read_pixels_rgba8().unwrap()
//     }

//     fn cached_shape_frame(left: Color, left_version: u64, policy: CachePolicy) -> BuiltFrame {
//         let mut ids = xui::render::RenderScene::new();
//         let root_source = ids.root();
//         let child_source = ids.insert_group();
//         let left_source = ids.insert_group();
//         let right_source = ids.insert_group();
//         let descriptor = LayerDescriptor {
//             cache_policy: policy,
//             force_offscreen: true,
//             ..LayerDescriptor::default()
//         };
//         let mut frame = layered_frame(
//             &descriptor,
//             Vec::new(),
//             vec![
//                 shape_draw(Rect::new(0.0, 0.0, 5.0, 10.0), left),
//                 shape_draw(Rect::new(5.0, 0.0, 5.0, 10.0), Color::rgb(0.0, 0.0, 1.0)),
//             ],
//         );
//         frame.layers[0].source = root_source;
//         frame.layers[1].source = child_source;
//         frame.layers[1].content_version.paint = left_version;
//         frame.layers[1].cache_policy = policy;
//         frame.layer_instances[0].source = child_source;
//         frame.layers[1].cache_id = Some(xui::render::LayerCacheId::Scene(child_source));
//         frame.live_layer_caches = (policy != CachePolicy::None)
//             .then_some(xui::render::LayerCacheId::Scene(child_source))
//             .into_iter()
//             .collect();
//         let BuiltItem::Draw(BuiltDraw::Shape(left_draw)) = &mut frame.layers[1].items[0] else {
//             unreachable!()
//         };
//         left_draw.common.source = left_source;
//         left_draw.common.content_version.paint = left_version;
//         let BuiltItem::Draw(BuiltDraw::Shape(right_draw)) = &mut frame.layers[1].items[1] else {
//             unreachable!()
//         };
//         right_draw.common.source = right_source;
//         frame
//     }

//     fn submit_headless_frame(backend: &mut TestBackend, frame: &BuiltFrame) -> Vec<u8> {
//         let mut text = TextHost::new(CosmicEngine::new(1.0));
//         <TestBackend as RenderBackend<TextHost<CosmicEngine>>>::begin_frame(
//             backend,
//             Size::new(10.0, 10.0),
//         )
//         .unwrap();
//         backend.submit(frame, &mut text).unwrap();
//         <TestBackend as RenderBackend<TextHost<CosmicEngine>>>::end_frame(backend).unwrap();
//         backend.read_pixels_rgba8().unwrap()
//     }

//     #[test]
//     fn cached_surface_is_reused_for_partial_repaint() {
//         let options = SkiaBackendOptions {
//             clear_color: Color::BLACK,
//             ..SkiaBackendOptions::default()
//         };
//         let mut backend = TestBackend::headless(1.0, options);
//         let first = cached_shape_frame(Color::rgb(1.0, 0.0, 0.0), 0, CachePolicy::Always);
//         submit_headless_frame(&mut backend, &first);
//         let initial = backend.layer_cache_stats();
//         assert_eq!(initial.misses, 1);
//         assert_eq!(initial.entries, 1);
//         assert_eq!(initial.full_updates, 1);

//         let mut second = first.clone();
//         second.layers[1].content_version.paint = 1;
//         let BuiltItem::Draw(BuiltDraw::Shape(left_draw)) = &mut second.layers[1].items[0] else {
//             unreachable!()
//         };
//         left_draw.common.content_version.paint = 1;
//         left_draw.primitive.fill = Some(ComputedColorStyle::Solid(Color::rgb(0.0, 1.0, 0.0)));
//         let partial_pixels = submit_headless_frame(&mut backend, &second);
//         let full_pixels = render_frame(&second);
//         assert_eq!(partial_pixels, full_pixels);
//         assert!(pixel(&partial_pixels, 10, 2, 5)[1] > 245);
//         assert!(pixel(&partial_pixels, 10, 7, 5)[2] > 245);
//         let updated = backend.layer_cache_stats();
//         assert_eq!(updated.hits, 1);
//         assert_eq!(updated.partial_updates, 1, "{updated:?}");
//         assert_eq!(updated.entries, 1);
//     }

//     #[test]
//     fn unchanged_frame_keeps_root_surface_generation() {
//         let mut backend = TestBackend::headless(1.0, SkiaBackendOptions::default());
//         let frame = cached_shape_frame(Color::WHITE, 0, CachePolicy::Always);
//         let first_pixels = submit_headless_frame(&mut backend, &frame);
//         let generation = backend.raster.as_mut().unwrap().generation_id();
//         let stats = backend.layer_cache_stats();

//         let second_pixels = submit_headless_frame(&mut backend, &frame);
//         assert_eq!(first_pixels, second_pixels);
//         assert_eq!(backend.raster.as_mut().unwrap().generation_id(), generation);
//         assert_eq!(backend.layer_cache_stats().hits, stats.hits);
//         assert_eq!(backend.layer_cache_stats().dirty_regions, 0);
//     }

//     #[test]
//     fn moved_draw_clears_old_bounds_and_matches_full_repaint() {
//         let mut backend = TestBackend::headless(
//             1.0,
//             SkiaBackendOptions {
//                 clear_color: Color::BLACK,
//                 ..SkiaBackendOptions::default()
//             },
//         );
//         let first = shape_frame(None);
//         submit_headless_frame(&mut backend, &first);

//         let mut moved = first.clone();
//         let BuiltItem::Draw(BuiltDraw::Shape(draw)) = &mut moved.layers[0].items[0] else {
//             unreachable!()
//         };
//         let bounds = Rect::new(5.0, 1.0, 4.0, 8.0);
//         draw.common.world_bounds = bounds;
//         draw.common.content_version.geometry = 1;
//         draw.primitive.bounds = bounds;
//         let partial = submit_headless_frame(&mut backend, &moved);
//         assert_eq!(partial, render_frame(&moved));
//         assert!(pixel(&partial, 10, 2, 5)[0] < 10);
//         assert!(pixel(&partial, 10, 7, 5)[0] > 245);
//     }

//     #[cfg(not(target_os = "macos"))]
//     #[test]
//     fn logical_damage_is_rounded_outward_in_physical_pixels() {
//         let rects = physical_damage_rects(
//             &DamageRegion::full(Rect::new(0.25, 0.5, 1.0, 1.0)),
//             2.0,
//             Size::new(10, 10),
//         );
//         assert_eq!(rects.len(), 1);
//         assert_eq!(rects[0].x, 0);
//         assert_eq!(rects[0].y, 1);
//         assert_eq!(rects[0].width.get(), 3);
//         assert_eq!(rects[0].height.get(), 2);
//     }

//     #[test]
//     fn auto_surfaces_obey_budget_while_always_surfaces_remain() {
//         let options = SkiaBackendOptions {
//             clear_color: Color::BLACK,
//             layer_cache_budget_bytes: 0,
//         };
//         let mut auto = TestBackend::headless(1.0, options);
//         submit_headless_frame(
//             &mut auto,
//             &cached_shape_frame(Color::WHITE, 0, CachePolicy::Auto),
//         );
//         assert_eq!(auto.layer_cache_stats().entries, 0);

//         let mut always = TestBackend::headless(1.0, options);
//         submit_headless_frame(
//             &mut always,
//             &cached_shape_frame(Color::WHITE, 0, CachePolicy::Always),
//         );
//         assert_eq!(always.layer_cache_stats().entries, 1);
//         assert_eq!(always.layer_cache_stats().resident_bytes, 400);
//     }

//     fn nested_backdrop_frame(isolation: BackdropIsolation) -> BuiltFrame {
//         let source = xui::render::RenderNodeId::default();
//         let outer_descriptor = LayerDescriptor {
//             force_offscreen: true,
//             ..LayerDescriptor::default()
//         };
//         let inner_descriptor = LayerDescriptor {
//             backdrop_style: Some(ComputedBackdropStyle {
//                 filters: Arc::from([ComputedBackdropFilter::Invert(1.0)]),
//                 ..ComputedBackdropStyle::default()
//             }),
//             force_offscreen: true,
//             ..LayerDescriptor::default()
//         };
//         let mut outer = layer_instance(&outer_descriptor);
//         outer.layer = BuiltLayerId(1);
//         let mut inner = layer_instance(&inner_descriptor);
//         inner.layer = BuiltLayerId(2);
//         inner.destination_prefix = Some(CompositePrefixId(1));
//         let bounds = Rect::new(0.0, 0.0, 10.0, 10.0);
//         let built_layer = |items, backdrop_isolation| BuiltLayer {
//             source,
//             content_bounds: bounds,
//             render_bounds: bounds,
//             content_version: ContentVersion::default(),
//             cache_id: None,
//             cache_policy: CachePolicy::None,
//             backdrop_isolation,
//             items,
//         };
//         BuiltFrame {
//             root_layer: BuiltLayerId(0),
//             layers: vec![
//                 built_layer(
//                     vec![
//                         shape_draw(bounds, Color::rgb(1.0, 0.0, 0.0)),
//                         BuiltItem::Layer(xui::render::BuiltLayerInstanceId(0)),
//                     ],
//                     BackdropIsolation::Isolate,
//                 ),
//                 built_layer(
//                     vec![BuiltItem::Layer(xui::render::BuiltLayerInstanceId(1))],
//                     isolation,
//                 ),
//                 built_layer(Vec::new(), BackdropIsolation::Isolate),
//             ],
//             layer_instances: vec![outer, inner],
//             composite_prefixes: vec![
//                 CompositePrefix {
//                     parent: None,
//                     local: SurfacePrefix {
//                         layer: BuiltLayerId(0),
//                         item_count: 1,
//                     },
//                     placement: None,
//                 },
//                 CompositePrefix {
//                     parent: Some(CompositePrefixId(0)),
//                     local: SurfacePrefix {
//                         layer: BuiltLayerId(1),
//                         item_count: 0,
//                     },
//                     placement: Some(xui::render::BuiltLayerInstanceId(0)),
//                 },
//             ],
//             clip_chains: Vec::new(),
//             live_layer_caches: Vec::new(),
//             scene_revision: 1,
//             properties_revision: 0,
//         }
//     }

//     #[test]
//     fn headless_backend_renders_at_physical_scale() {
//         let mut backend = TestBackend::headless(2.0, SkiaBackendOptions::default());
//         let mut text = TextHost::new(CosmicEngine::new(2.0));
//         <TestBackend as RenderBackend<TextHost<CosmicEngine>>>::begin_frame(
//             &mut backend,
//             Size::new(10.0, 10.0),
//         )
//         .unwrap();
//         backend.submit(&shape_frame(None), &mut text).unwrap();
//         <TestBackend as RenderBackend<TextHost<CosmicEngine>>>::end_frame(&mut backend).unwrap();

//         assert_eq!(backend.frame_size_px(), Size::new(20, 20));
//         let pixels = backend.read_pixels_rgba8().unwrap();
//         let inside = pixel(&pixels, 20, 10, 10);
//         assert!(inside[0] > 245 && inside[1] < 10 && inside[2] < 10);
//         assert!(<TestBackend as RenderBackend<TextHost<CosmicEngine>>>::did_present(&backend));
//     }

//     #[test]
//     fn clip_chain_limits_shape_output() {
//         let mut backend = TestBackend::headless(1.0, SkiaBackendOptions::default());
//         let mut text = TextHost::new(CosmicEngine::new(1.0));
//         <TestBackend as RenderBackend<TextHost<CosmicEngine>>>::begin_frame(
//             &mut backend,
//             Size::new(10.0, 10.0),
//         )
//         .unwrap();
//         backend
//             .submit(
//                 &shape_frame(Some(Rect::new(0.0, 0.0, 5.0, 10.0))),
//                 &mut text,
//             )
//             .unwrap();
//         <TestBackend as RenderBackend<TextHost<CosmicEngine>>>::end_frame(&mut backend).unwrap();

//         let pixels = backend.read_pixels_rgba8().unwrap();
//         assert!(pixel(&pixels, 10, 2, 5)[0] > 245);
//         let outside = pixel(&pixels, 10, 8, 5);
//         assert!(outside[0] < 40 && outside[1] < 40 && outside[2] < 40);
//     }

//     #[test]
//     fn image_rotation_and_flips_rearrange_pixels() {
//         let data = ImageData::rgba8(Size::new(2, 1), [255, 0, 0, 255, 0, 255, 0, 255]);
//         let (rotated, width, height) = transform_image_pixels(
//             &data,
//             ImageTransform {
//                 rotate: ImageRotation::Deg90,
//                 ..ImageTransform::default()
//             },
//         );
//         assert_eq!((width, height), (1, 2));
//         assert_eq!(&rotated[..4], &[255, 0, 0, 255]);
//         assert_eq!(&rotated[4..], &[0, 255, 0, 255]);

//         let (flipped, _, _) = transform_image_pixels(
//             &data,
//             ImageTransform {
//                 flip_x: true,
//                 ..ImageTransform::default()
//             },
//         );
//         assert_eq!(&flipped[..4], &[0, 255, 0, 255]);
//     }

//     #[test]
//     fn common_layer_effects_lower_to_an_executable_plan() {
//         let descriptor = LayerDescriptor {
//             effects: Arc::from([
//                 ComputedEffect::Blur {
//                     sigma_x: 1.5,
//                     sigma_y: 2.0,
//                     quality: FilterQuality::Medium,
//                 },
//                 ComputedEffect::ColorMatrix([
//                     1.0, 0.0, 0.0, 0.0, 0.0, // red
//                     0.0, 1.0, 0.0, 0.0, 0.0, // green
//                     0.0, 0.0, 1.0, 0.0, 0.0, // blue
//                     0.0, 0.0, 0.0, 1.0, 0.0, // alpha
//                 ]),
//                 ComputedEffect::DropShadow {
//                     color: Color::rgba(0.0, 0.0, 0.0, 0.75),
//                     offset: Point::new(2.0, 3.0),
//                     sigma_x: 2.0,
//                     sigma_y: 2.0,
//                     spread: 1.0,
//                     quality: FilterQuality::High,
//                 },
//             ]),
//             force_offscreen: true,
//             ..LayerDescriptor::default()
//         };
//         let instance = layer_instance(&descriptor);
//         let plan = instance
//             .render_program
//             .program()
//             .instantiate(&LayerPlanContext {
//                 backdrop_source_bounds: Rect::new(0.0, 0.0, 20.0, 20.0),
//                 parent_destination_bounds: Rect::new(0.0, 0.0, 20.0, 20.0),
//                 composite_clip_bounds: Some(Rect::new(0.0, 0.0, 10.0, 10.0)),
//                 layer_content_bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
//                 backdrop_bounds: None,
//                 composite: instance.composite,
//                 scale_factor: 1.0,
//                 color_texture_class: TextureClass::LINEAR_COLOR,
//                 external_aliasing: ExternalAliasing::Distinct,
//                 limits: PlanLimits::default(),
//             })
//             .unwrap();
//         assert!(plan.passes().len() >= 5);
//     }

//     #[test]
//     fn custom_effects_and_composite_blender_compile() {
//         let mut backend = TestBackend::headless(1.0, SkiaBackendOptions::default());
//         assert!(
//             backend
//                 .runtime_filter("pixelate", PIXELATE_SKSL, &[("block", &[4.0, 4.0])])
//                 .is_ok()
//         );
//         assert!(
//             backend
//                 .runtime_filter(
//                     "refraction",
//                     REFRACTION_SKSL,
//                     &[("center", &[5.0, 5.0]), ("amount", &[2.0, 1.0])],
//                 )
//                 .is_ok()
//         );
//         assert!(
//             backend
//                 .runtime_filter(
//                     "chromatic-aberration",
//                     CHROMATIC_ABERRATION_SKSL,
//                     &[("offset", &[1.0, 0.0])],
//                 )
//                 .is_ok()
//         );
//         assert!(
//             backend
//                 .runtime_blender(BlendMode::Multiply, CompositeOperator::DstOver)
//                 .is_ok()
//         );
//     }

//     #[test]
//     fn image_mask_is_accepted_and_backdrop_requires_a_valid_prefix() {
//         let mut frame = shape_frame(None);
//         let backend = TestBackend::headless(1.0, SkiaBackendOptions::default());

//         let image_mask = LayerDescriptor {
//             effects: Arc::from([ComputedEffect::ImageMask {
//                 image: ImageKey::UserProvided(7),
//                 data: ImageData::rgba8(Size::new(1, 1), [255, 255, 255, 255]),
//                 bounds: Rect::new(0.0, 0.0, 1.0, 1.0),
//             }]),
//             ..LayerDescriptor::default()
//         };
//         frame.layer_instances = vec![layer_instance(&image_mask)];
//         assert!(backend.validate_frame(&frame).is_ok());

//         let plain_backdrop = LayerDescriptor {
//             backdrop_style: Some(ComputedBackdropStyle {
//                 filters: Arc::from([ComputedBackdropFilter::Blur {
//                     sigma_x: 2.0,
//                     sigma_y: 2.0,
//                     quality: FilterQuality::Medium,
//                 }]),
//                 ..ComputedBackdropStyle::default()
//             }),
//             ..LayerDescriptor::default()
//         };
//         frame.layer_instances = vec![layer_instance(&plain_backdrop)];
//         assert!(matches!(
//             backend.validate_frame(&frame),
//             Err(SkiaBackendError::InvalidFrame(_))
//         ));
//     }

//     #[test]
//     fn image_mask_is_executed_by_the_offscreen_plan() {
//         let descriptor = LayerDescriptor {
//             effects: Arc::from([ComputedEffect::ImageMask {
//                 image: ImageKey::UserProvided(17),
//                 data: ImageData::rgba8(Size::new(2, 1), [255, 255, 255, 255, 255, 255, 255, 0]),
//                 bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
//             }]),
//             force_offscreen: true,
//             ..LayerDescriptor::default()
//         };
//         let frame = layered_frame(
//             &descriptor,
//             vec![shape_draw(
//                 Rect::new(0.0, 0.0, 10.0, 10.0),
//                 Color::rgb(0.0, 0.0, 1.0),
//             )],
//             vec![shape_draw(
//                 Rect::new(0.0, 0.0, 10.0, 10.0),
//                 Color::rgb(1.0, 0.0, 0.0),
//             )],
//         );
//         let pixels = render_frame(&frame);
//         let left = pixel(&pixels, 10, 2, 5);
//         let right = pixel(&pixels, 10, 8, 5);
//         assert!(left[0] > 200 && left[2] < 40, "left={left:?}");
//         assert!(right[2] > 200 && right[0] < 40, "right={right:?}");
//     }

//     #[test]
//     fn frame_stats_expose_render_graph_and_surface_work() {
//         let descriptor = LayerDescriptor {
//             effects: Arc::from([
//                 ComputedEffect::Blur {
//                     sigma_x: 2.0,
//                     sigma_y: 2.0,
//                     quality: FilterQuality::Medium,
//                 },
//                 ComputedEffect::ColorMatrix([
//                     1.0, 0.0, 0.0, 0.0, 0.0, // red
//                     0.0, 1.0, 0.0, 0.0, 0.0, // green
//                     0.0, 0.0, 1.0, 0.0, 0.0, // blue
//                     0.0, 0.0, 0.0, 1.0, 0.0, // alpha
//                 ]),
//                 ComputedEffect::DropShadow {
//                     color: Color::rgba(0.0, 0.0, 0.0, 0.5),
//                     offset: Point::new(1.0, 1.0),
//                     sigma_x: 1.0,
//                     sigma_y: 1.0,
//                     spread: 1.0,
//                     quality: FilterQuality::Medium,
//                 },
//             ]),
//             force_offscreen: true,
//             ..LayerDescriptor::default()
//         };
//         let frame = layered_frame(
//             &descriptor,
//             Vec::new(),
//             vec![shape_draw(Rect::new(0.0, 0.0, 10.0, 10.0), Color::WHITE)],
//         );
//         let mut backend = TestBackend::headless(1.0, SkiaBackendOptions::default());
//         submit_headless_frame(&mut backend, &frame);

//         let stats = backend.frame_stats();
//         assert_eq!(stats.frame_index, 1);
//         assert!(stats.root_damage_rects > 0, "{stats:?}");
//         assert_eq!(stats.layer_draws, 2);
//         assert_eq!(stats.primitive_draws, 1);
//         assert_eq!(stats.render_plans, 1);
//         assert!(stats.render_passes >= 3, "{stats:?}");
//         assert!(stats.planned_transient_resources > 0, "{stats:?}");
//         assert!(stats.planned_transient_slots > 0, "{stats:?}");
//         assert_eq!(
//             stats.transient_surface_allocations,
//             stats.planned_transient_slots
//         );
//         assert!(stats.transient_surface_reuses > 0, "{stats:?}");
//         assert!(stats.offscreen_surface_allocations > 0, "{stats:?}");
//         assert!(stats.image_snapshots > 0, "{stats:?}");
//         assert_eq!(stats.backdrop_materializations, 0, "{stats:?}");
//         assert_eq!(stats.backdrop_materializations_avoided, 1, "{stats:?}");
//     }

//     #[test]
//     fn pixelate_backdrop_samples_only_the_prior_destination() {
//         let descriptor = LayerDescriptor {
//             backdrop_style: Some(ComputedBackdropStyle {
//                 filters: Arc::from([ComputedBackdropFilter::Pixelate {
//                     size: Size::new(4.0, 4.0),
//                 }]),
//                 ..ComputedBackdropStyle::default()
//             }),
//             force_offscreen: true,
//             ..LayerDescriptor::default()
//         };
//         let frame = layered_frame(
//             &descriptor,
//             vec![
//                 shape_draw(Rect::new(0.0, 0.0, 5.0, 10.0), Color::rgb(1.0, 0.0, 0.0)),
//                 shape_draw(Rect::new(5.0, 0.0, 5.0, 10.0), Color::rgb(0.0, 0.0, 1.0)),
//             ],
//             Vec::new(),
//         );
//         let pixels = render_frame(&frame);
//         let left = pixel(&pixels, 10, 1, 5);
//         let right = pixel(&pixels, 10, 8, 5);
//         assert!(left[0] > 180 && left[2] < 80, "left={left:?}");
//         assert!(right[2] > 180 && right[0] < 80, "right={right:?}");
//     }

//     #[test]
//     fn nested_backdrop_respects_passthrough_and_isolation() {
//         let passthrough_frame = nested_backdrop_frame(BackdropIsolation::Passthrough);
//         let isolated_frame = nested_backdrop_frame(BackdropIsolation::Isolate);
//         assert!(BackdropRequirements::for_frame(&passthrough_frame).layer(BuiltLayerId(0)));
//         assert!(!BackdropRequirements::for_frame(&isolated_frame).layer(BuiltLayerId(0)));

//         let passthrough = render_frame(&passthrough_frame);
//         let isolated = render_frame(&isolated_frame);
//         let passthrough = pixel(&passthrough, 10, 5, 5);
//         let isolated = pixel(&isolated, 10, 5, 5);
//         assert!(
//             passthrough[1] > 180 && passthrough[2] > 180 && passthrough[0] < 80,
//             "passthrough={passthrough:?}"
//         );
//         assert!(
//             isolated[0] > 180 && isolated[1] < 80 && isolated[2] < 80,
//             "isolated={isolated:?}"
//         );
//     }

//     #[test]
//     fn refraction_and_chromatic_aberration_execute_on_cpu_raster() {
//         let descriptor = LayerDescriptor {
//             backdrop_style: Some(ComputedBackdropStyle {
//                 filters: Arc::from([
//                     ComputedBackdropFilter::Refraction {
//                         strength: 2.0,
//                         chromatic_aberration: 1.0,
//                     },
//                     ComputedBackdropFilter::ChromaticAberration { offset: [1.0, 0.0] },
//                 ]),
//                 ..ComputedBackdropStyle::default()
//             }),
//             force_offscreen: true,
//             ..LayerDescriptor::default()
//         };
//         let frame = layered_frame(
//             &descriptor,
//             vec![
//                 shape_draw(Rect::new(0.0, 0.0, 5.0, 10.0), Color::rgb(1.0, 0.0, 0.0)),
//                 shape_draw(Rect::new(5.0, 0.0, 5.0, 10.0), Color::rgb(0.0, 1.0, 1.0)),
//             ],
//             Vec::new(),
//         );
//         let pixels = render_frame(&frame);
//         let center = pixel(&pixels, 10, 5, 5);
//         assert!(center[3] > 200, "center={center:?}");
//         assert!(
//             center[0] != center[1] || center[1] != center[2],
//             "center={center:?}"
//         );
//     }

//     #[test]
//     fn artistic_blend_can_be_combined_with_src_operator() {
//         let descriptor = LayerDescriptor {
//             composite: CompositeStyle {
//                 blend_mode: BlendMode::Multiply,
//                 operator: CompositeOperator::Src,
//                 ..CompositeStyle::default()
//             },
//             force_offscreen: true,
//             ..LayerDescriptor::default()
//         };
//         let frame = layered_frame(
//             &descriptor,
//             vec![shape_draw(
//                 Rect::new(0.0, 0.0, 10.0, 10.0),
//                 Color::rgb(0.0, 0.0, 1.0),
//             )],
//             vec![shape_draw(
//                 Rect::new(0.0, 0.0, 10.0, 10.0),
//                 Color::rgb(1.0, 0.0, 0.0),
//             )],
//         );
//         let pixels = render_frame(&frame);
//         let center = pixel(&pixels, 10, 5, 5);
//         assert!(
//             center[0] < 40 && center[1] < 40 && center[2] < 40 && center[3] > 240,
//             "center={center:?}"
//         );
//     }

//     #[test]
//     fn missing_keyed_backdrop_mask_is_structured_error() {
//         let key = ImageKey::UserProvided(404);
//         let descriptor = LayerDescriptor {
//             backdrop_style: Some(ComputedBackdropStyle {
//                 mask: ComputedBackdropMask::AlphaTexture {
//                     texture: key.clone(),
//                     transform: Affine::scale(10.0, 10.0),
//                 },
//                 ..ComputedBackdropStyle::default()
//             }),
//             force_offscreen: true,
//             ..LayerDescriptor::default()
//         };
//         let frame = layered_frame(
//             &descriptor,
//             vec![shape_draw(Rect::new(0.0, 0.0, 10.0, 10.0), Color::WHITE)],
//             Vec::new(),
//         );
//         let mut backend = TestBackend::headless(1.0, SkiaBackendOptions::default());
//         let mut text = TextHost::new(CosmicEngine::new(1.0));
//         <TestBackend as RenderBackend<TextHost<CosmicEngine>>>::begin_frame(
//             &mut backend,
//             Size::new(10.0, 10.0),
//         )
//         .unwrap();
//         assert!(matches!(
//             backend.submit(&frame, &mut text),
//             Err(SkiaBackendError::MissingMaskImage(value)) if value == key
//         ));
//         assert!(!<TestBackend as RenderBackend<TextHost<CosmicEngine>>>::did_present(&backend));
//     }
// }
