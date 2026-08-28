use std::{ops::Range, sync::Arc};

use moka::sync::Cache;
use vello::{
    AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene,
    kurbo::{Affine as KurboAffine, BezPath, Cap, Join, Rect as KurboRect, Stroke},
    peniko::{
        Fill,
        color::{AlphaColor, Srgb, palette},
    },
};
use wgpu::util::DeviceExt;
use xui_interface::{
    Affine, Color, FillRule, LineCap, LineJoin, PathData, PathDataId, PathFill, PathSegment,
    PathStroke, Point, Rect, VectorCommand, VectorScene, VectorSceneId,
};

use crate::wgpu::{SCENE_FORMAT, SCENE_SAMPLE_COUNT, physical_scissor};

const VECTOR_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2];

#[derive(Debug, Clone)]
pub struct VectorDrawRecord {
    pub scene: VectorScene,
    pub transform: Affine,
    pub opacity: f32,
    pub clip: Rect,
}

impl VectorDrawRecord {
    fn visible_bounds(&self) -> Option<Rect> {
        let bounds = self.transform.transform_bounds(self.scene.bounds());
        intersect_rect(
            Rect::new(bounds.x(), bounds.y(), bounds.width(), bounds.height()),
            self.clip,
        )
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct VectorVertex {
    position: [f32; 2],
    uv: [f32; 2],
}

impl VectorVertex {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VECTOR_VERTEX_ATTRIBUTES,
        }
    }
}

struct VectorTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: (u32, u32),
}

struct VectorComposite {
    vertices: [VectorVertex; 6],
    bind_group: Arc<wgpu::BindGroup>,
    clip: Rect,
}

#[derive(Clone)]
enum CompiledVectorCommand {
    FillPath {
        path: Arc<BezPath>,
        transform: Affine,
        fill: PathFill,
    },
    StrokePath {
        path: Arc<BezPath>,
        transform: Affine,
        stroke: PathStroke,
    },
}

pub struct VectorRenderer {
    renderer: Renderer,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: usize,
    composites: Vec<VectorComposite>,
    active_targets: Vec<VectorTarget>,
    available_targets: Vec<VectorTarget>,
    paths: Cache<PathDataId, Arc<BezPath>>,
    scenes: Cache<VectorSceneId, Arc<[CompiledVectorCommand]>>,
}

impl VectorRenderer {
    pub fn new(
        device: &wgpu::Device,
        ui_layout: &wgpu::BindGroupLayout,
    ) -> Result<Self, vello::Error> {
        let renderer = Renderer::new(
            device,
            RendererOptions {
                antialiasing_support: AaSupport::area_only(),
                ..RendererOptions::default()
            },
        )?;
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("xui vector bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("xui vector sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xui vector composite shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/vector.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xui vector pipeline layout"),
            bind_group_layouts: &[Some(ui_layout), Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xui vector pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[VectorVertex::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: SCENE_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: SCENE_SAMPLE_COUNT,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("xui empty vector vertices"),
            size: 4,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            renderer,
            pipeline,
            bind_group_layout,
            sampler,
            vertex_buffer,
            vertex_capacity: 0,
            composites: Vec::new(),
            active_targets: Vec::new(),
            available_targets: Vec::new(),
            paths: Cache::new(4096),
            scenes: Cache::new(1024),
        })
    }

    pub fn begin_frame(&mut self) {
        self.composites.clear();
        self.available_targets.clear();
        self.available_targets.append(&mut self.active_targets);
    }

    pub fn rasterize_run(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        records: &[VectorDrawRecord],
        range: Range<usize>,
        scale_factor: f32,
    ) -> Result<Option<usize>, vello::Error> {
        let scale_factor = scale_factor.max(f32::EPSILON);
        let Some(bounds) = records[range.clone()]
            .iter()
            .filter(|record| record.opacity > 0.0 && !record.scene.is_empty())
            .filter_map(VectorDrawRecord::visible_bounds)
            .reduce(Rect::union)
        else {
            return Ok(None);
        };
        let physical_x0 = (bounds.x * scale_factor).floor();
        let physical_y0 = (bounds.y * scale_factor).floor();
        let physical_x1 = ((bounds.x + bounds.width) * scale_factor).ceil();
        let physical_y1 = ((bounds.y + bounds.height) * scale_factor).ceil();
        let width = (physical_x1 - physical_x0).max(0.0) as u32;
        let height = (physical_y1 - physical_y0).max(0.0) as u32;
        if width == 0 || height == 0 {
            return Ok(None);
        }

        let target = self.take_target(device, (width, height));
        let mut scene = Scene::new();
        let target_transform = Affine::scale(scale_factor, scale_factor)
            .then(Affine::translate(-physical_x0, -physical_y0));
        for record in &records[range] {
            if record.opacity <= 0.0 || record.scene.is_empty() {
                continue;
            }
            let Some(clip) = record.visible_bounds() else {
                continue;
            };
            let clip = target_transform.transform_rect(clip);
            let clip = KurboRect::new(
                clip.x as f64,
                clip.y as f64,
                (clip.x + clip.width) as f64,
                (clip.y + clip.height) as f64,
            );
            scene.push_clip_layer(Fill::NonZero, KurboAffine::IDENTITY, &clip);
            let outer = record.transform.then(target_transform);
            let compiled = self.compiled_scene(&record.scene);
            for command in compiled.iter() {
                encode_command(&mut scene, command, outer, record.opacity);
            }
            scene.pop_layer();
        }
        self.renderer.render_to_texture(
            device,
            queue,
            &scene,
            &target.view,
            &RenderParams {
                base_color: palette::css::TRANSPARENT,
                width,
                height,
                antialiasing_method: AaConfig::Area,
            },
        )?;

        let bind_group = Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xui vector bind group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&target.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        }));
        let logical_bounds = Rect::new(
            physical_x0 / scale_factor,
            physical_y0 / scale_factor,
            width as f32 / scale_factor,
            height as f32 / scale_factor,
        );
        let index = self.composites.len();
        self.composites.push(VectorComposite {
            vertices: quad_vertices(logical_bounds),
            bind_group,
            clip: logical_bounds,
        });
        self.active_targets.push(target);
        Ok(Some(index))
    }

    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let vertices: Vec<_> = self
            .composites
            .iter()
            .flat_map(|composite| composite.vertices)
            .collect();
        if vertices.is_empty() {
            return;
        }
        let bytes = bytemuck::cast_slice(&vertices);
        if bytes.len() > self.vertex_capacity {
            self.vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("xui vector vertices"),
                contents: bytes,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
            self.vertex_capacity = bytes.len();
        } else {
            queue.write_buffer(&self.vertex_buffer, 0, bytes);
        }
    }

    pub fn render(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        ui_bind_group: &wgpu::BindGroup,
        composite: Option<usize>,
        scale_factor: f32,
        target_size: (u32, u32),
    ) {
        let Some(index) = composite else {
            return;
        };
        let composite = &self.composites[index];
        let Some((x, y, width, height)) =
            physical_scissor(composite.clip, scale_factor, target_size)
        else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, ui_bind_group, &[]);
        pass.set_bind_group(1, composite.bind_group.as_ref(), &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_scissor_rect(x, y, width, height);
        let start = (index * 6) as u32;
        pass.draw(start..start + 6, 0..1);
    }

    fn take_target(&mut self, device: &wgpu::Device, size: (u32, u32)) -> VectorTarget {
        if let Some(index) = self
            .available_targets
            .iter()
            .position(|target| target.size == size)
        {
            return self.available_targets.swap_remove(index);
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("xui vector target"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        VectorTarget {
            _texture: texture,
            view,
            size,
        }
    }

    fn compiled_scene(&self, scene: &VectorScene) -> Arc<[CompiledVectorCommand]> {
        if let Some(compiled) = self.scenes.get(&scene.id()) {
            return compiled;
        }
        let compiled: Arc<[CompiledVectorCommand]> = scene
            .commands()
            .iter()
            .filter_map(|command| match command {
                VectorCommand::FillPath {
                    path,
                    transform,
                    fill,
                } => Some(CompiledVectorCommand::FillPath {
                    path: self.compiled_path(path),
                    transform: *transform,
                    fill: *fill,
                }),
                VectorCommand::StrokePath {
                    path,
                    transform,
                    stroke,
                } => Some(CompiledVectorCommand::StrokePath {
                    path: self.compiled_path(path),
                    transform: *transform,
                    stroke: *stroke,
                }),
                // Shapes and text never arrive here: the canvas lowers both to
                // their own primitives before a vector run is formed, so a
                // vector scene is paths only by construction.
                VectorCommand::Shape { .. } | VectorCommand::TextBox { .. } => None,
            })
            .collect::<Vec<_>>()
            .into();
        self.scenes.insert(scene.id(), Arc::clone(&compiled));
        compiled
    }

    fn compiled_path(&self, path: &PathData) -> Arc<BezPath> {
        if let Some(compiled) = self.paths.get(&path.id()) {
            return compiled;
        }
        let compiled = Arc::new(kurbo_path(path));
        self.paths.insert(path.id(), Arc::clone(&compiled));
        compiled
    }
}

fn quad_vertices(bounds: Rect) -> [VectorVertex; 6] {
    let p0 = [bounds.x, bounds.y];
    let p1 = [bounds.x + bounds.width, bounds.y];
    let p2 = [bounds.x, bounds.y + bounds.height];
    let p3 = [bounds.x + bounds.width, bounds.y + bounds.height];
    let vertex = |position, uv| VectorVertex { position, uv };
    [
        vertex(p0, [0.0, 0.0]),
        vertex(p1, [1.0, 0.0]),
        vertex(p2, [0.0, 1.0]),
        vertex(p2, [0.0, 1.0]),
        vertex(p1, [1.0, 0.0]),
        vertex(p3, [1.0, 1.0]),
    ]
}

fn encode_command(scene: &mut Scene, command: &CompiledVectorCommand, outer: Affine, opacity: f32) {
    match command {
        CompiledVectorCommand::FillPath {
            path,
            transform,
            fill,
        } => {
            let color = color(fill.color, opacity);
            let fill = match fill.rule {
                FillRule::NonZero => Fill::NonZero,
                FillRule::EvenOdd => Fill::EvenOdd,
            };
            scene.fill(
                fill,
                kurbo_affine(transform.then(outer)),
                color,
                None,
                path.as_ref(),
            );
        }
        CompiledVectorCommand::StrokePath {
            path,
            transform,
            stroke,
        } if stroke.width > 0.0 => {
            let cap = match stroke.cap {
                LineCap::Butt => Cap::Butt,
                LineCap::Square => Cap::Square,
                LineCap::Round => Cap::Round,
            };
            let join = match stroke.join {
                LineJoin::Miter => Join::Miter,
                LineJoin::Bevel => Join::Bevel,
                LineJoin::Round => Join::Round,
            };
            let mut style = Stroke::new(stroke.width as f64)
                .with_caps(cap)
                .with_join(join);
            if let Some(dash) = stroke.effective_dash() {
                style = style.with_dashes(
                    dash.offset as f64,
                    dash.intervals().iter().map(|value| *value as f64),
                );
            }
            scene.stroke(
                &style,
                kurbo_affine(transform.then(outer)),
                color(stroke.color, opacity),
                None,
                path.as_ref(),
            );
        }
        CompiledVectorCommand::StrokePath { .. } => {}
    }
}

fn color(color: Color, opacity: f32) -> AlphaColor<Srgb> {
    AlphaColor::new([color.r, color.g, color.b, color.a * opacity.clamp(0.0, 1.0)])
}

fn kurbo_affine(value: Affine) -> KurboAffine {
    KurboAffine::new([
        value.xx as f64,
        value.yx as f64,
        value.xy as f64,
        value.yy as f64,
        value.dx as f64,
        value.dy as f64,
    ])
}

fn kurbo_path(path: &PathData) -> BezPath {
    let mut result = BezPath::new();
    for segment in path.segments() {
        match *segment {
            PathSegment::MoveTo(point) => result.move_to(kurbo_point(point)),
            PathSegment::LineTo(point) => result.line_to(kurbo_point(point)),
            PathSegment::QuadraticTo { control, to } => {
                result.quad_to(kurbo_point(control), kurbo_point(to));
            }
            PathSegment::CubicTo {
                control1,
                control2,
                to,
            } => result.curve_to(
                kurbo_point(control1),
                kurbo_point(control2),
                kurbo_point(to),
            ),
            PathSegment::Close => result.close_path(),
        }
    }
    result
}

fn kurbo_point(point: Point) -> vello::kurbo::Point {
    vello::kurbo::Point::new(point.x as f64, point.y as f64)
}

fn intersect_rect(a: Rect, b: Rect) -> Option<Rect> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.width).min(b.x + b.width);
    let y1 = (a.y + a.height).min(b.y + b.height);
    (x1 > x0 && y1 > y0).then(|| Rect::new(x0, y0, x1 - x0, y1 - y0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use xui_interface::PathBuilder;

    #[test]
    fn path_conversion_preserves_every_segment() {
        let mut path = PathBuilder::new();
        path.move_to(Point::new(1.0, 2.0))
            .line_to(Point::new(3.0, 4.0))
            .quadratic_to(Point::new(5.0, 6.0), Point::new(7.0, 8.0))
            .cubic_to(
                Point::new(9.0, 10.0),
                Point::new(11.0, 12.0),
                Point::new(13.0, 14.0),
            )
            .close();
        assert_eq!(kurbo_path(&path.build()).elements().len(), 5);
    }

    #[test]
    fn vector_composite_shader_parses_as_wgsl() {
        naga::front::wgsl::parse_str(include_str!("../shaders/vector.wgsl")).unwrap();
    }
}
