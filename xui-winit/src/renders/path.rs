use std::ops::Range;
use std::sync::Arc;

use lyon_path::math::point;
use lyon_tessellation::geometry_builder::{BuffersBuilder, VertexBuffers};
use lyon_tessellation::{
    FillOptions, FillTessellator, FillVertex, StrokeOptions, StrokeTessellator, StrokeVertex,
};
use wgpu::util::DeviceExt;
use xui_interface::{
    Affine, Color, FillRule, LineCap, LineJoin, PathData, PathDataId, PathFill, PathSegment,
    PathStroke, Rect,
};

use crate::wgpu::{SCENE_FORMAT, SCENE_SAMPLE_COUNT};

#[derive(Debug, Clone)]
pub struct PathDrawRecord {
    pub path: PathData,
    pub transform: Affine,
    pub fill: Option<PathFill>,
    pub stroke: Option<PathStroke>,
    pub clip: Rect,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
enum GeometryKey {
    Fill(PathDataId, FillRule),
    Stroke(PathDataId, u32, LineCap, LineJoin),
}

#[derive(Default)]
struct Mesh {
    vertices: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PathVertex {
    position: [f32; 2],
    color: [f32; 4],
}

struct DrawBatch {
    indices: Range<u32>,
    clip: Rect,
}

pub struct PathRenderer {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    vertex_capacity: usize,
    index_capacity: usize,
    index_count: u32,
    batches: Vec<DrawBatch>,
    record_batches: Vec<Range<usize>>,
    meshes: moka::sync::Cache<GeometryKey, Arc<Mesh>>,
}

impl PathRenderer {
    pub fn new(device: &wgpu::Device, ui_layout: &wgpu::BindGroupLayout) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xui path shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/path.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xui path pipeline layout"),
            bind_group_layouts: &[Some(ui_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xui path pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<PathVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
                }],
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
        let empty = || wgpu::BufferDescriptor {
            label: Some("xui empty path buffer"),
            size: 4,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::INDEX
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        };
        Self {
            pipeline,
            vertex_buffer: device.create_buffer(&empty()),
            index_buffer: device.create_buffer(&empty()),
            vertex_capacity: 0,
            index_capacity: 0,
            index_count: 0,
            batches: Vec::new(),
            record_batches: Vec::new(),
            meshes: moka::sync::Cache::builder().max_capacity(2048).build(),
        }
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        records: &[PathDrawRecord],
    ) {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut batches = Vec::new();
        let mut record_batches = Vec::with_capacity(records.len());

        for record in records {
            let batch_start = batches.len();
            if record.path.is_empty() || record.clip.width <= 0.0 || record.clip.height <= 0.0 {
                record_batches.push(batch_start..batch_start);
                continue;
            }
            if let Some(fill) = record.fill.filter(|fill| fill.color.a > 0.0) {
                let key = GeometryKey::Fill(record.path.id(), fill.rule);
                let mesh = self.meshes.get(&key).unwrap_or_else(|| {
                    let mesh = Arc::new(tessellate_fill(&record.path, fill.rule));
                    self.meshes.insert(key, Arc::clone(&mesh));
                    mesh
                });
                append_mesh(
                    &mesh,
                    record.transform,
                    fill.color,
                    record.clip,
                    &mut vertices,
                    &mut indices,
                    &mut batches,
                );
            }
            if let Some(stroke) = record
                .stroke
                .filter(|stroke| stroke.color.a > 0.0 && stroke.width > 0.0)
            {
                let key = GeometryKey::Stroke(
                    record.path.id(),
                    stroke.width.to_bits(),
                    stroke.cap,
                    stroke.join,
                );
                let mesh = self.meshes.get(&key).unwrap_or_else(|| {
                    let mesh = Arc::new(tessellate_stroke(&record.path, stroke));
                    self.meshes.insert(key, Arc::clone(&mesh));
                    mesh
                });
                append_mesh(
                    &mesh,
                    record.transform,
                    stroke.color,
                    record.clip,
                    &mut vertices,
                    &mut indices,
                    &mut batches,
                );
            }
            record_batches.push(batch_start..batches.len());
        }

        self.index_count = indices.len() as u32;
        self.batches = batches;
        self.record_batches = record_batches;
        if vertices.is_empty() {
            return;
        }
        let vertex_bytes = bytemuck::cast_slice(&vertices);
        if vertex_bytes.len() > self.vertex_capacity {
            self.vertex_capacity = vertex_bytes.len();
            self.vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("xui path vertices"),
                contents: vertex_bytes,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
        } else {
            queue.write_buffer(&self.vertex_buffer, 0, vertex_bytes);
        }
        let index_bytes = bytemuck::cast_slice(&indices);
        if index_bytes.len() > self.index_capacity {
            self.index_capacity = index_bytes.len();
            self.index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("xui path indices"),
                contents: index_bytes,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            });
        } else {
            queue.write_buffer(&self.index_buffer, 0, index_bytes);
        }
    }

    pub fn render(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        ui_bind_group: &wgpu::BindGroup,
        scale_factor: f32,
        target_size: (u32, u32),
    ) {
        self.render_range(
            pass,
            ui_bind_group,
            0..self.record_batches.len(),
            scale_factor,
            target_size,
        );
    }

    pub fn render_range(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        ui_bind_group: &wgpu::BindGroup,
        records: Range<usize>,
        scale_factor: f32,
        target_size: (u32, u32),
    ) {
        if self.index_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, ui_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        let batch_start = records
            .clone()
            .next()
            .map(|record| self.record_batches[record].start)
            .unwrap_or(0);
        let batch_end = records
            .last()
            .map(|record| self.record_batches[record].end)
            .unwrap_or(batch_start);
        for batch in &self.batches[batch_start..batch_end] {
            if let Some((x, y, width, height)) =
                crate::wgpu::physical_scissor(batch.clip, scale_factor, target_size)
            {
                pass.set_scissor_rect(x, y, width, height);
                pass.draw_indexed(batch.indices.clone(), 0, 0..1);
            }
        }
    }
}

fn append_mesh(
    mesh: &Mesh,
    transform: Affine,
    color: Color,
    clip: Rect,
    vertices: &mut Vec<PathVertex>,
    indices: &mut Vec<u32>,
    batches: &mut Vec<DrawBatch>,
) {
    if mesh.indices.is_empty() {
        return;
    }
    let vertex_base = vertices.len() as u32;
    let index_start = indices.len() as u32;
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    vertices.extend(mesh.vertices.iter().map(|position| {
        let p = transform.transform_point(xui_interface::Point::new(position[0], position[1]));
        min_x = min_x.min(p.x);
        min_y = min_y.min(p.y);
        max_x = max_x.max(p.x);
        max_y = max_y.max(p.y);
        PathVertex {
            position: [p.x, p.y],
            color: [color.r, color.g, color.b, color.a],
        }
    }));
    if !min_x.is_finite()
        || max_x <= clip.x
        || max_y <= clip.y
        || min_x >= clip.x + clip.width
        || min_y >= clip.y + clip.height
    {
        vertices.truncate(vertex_base as usize);
        return;
    }
    indices.extend(mesh.indices.iter().map(|index| vertex_base + index));
    batches.push(DrawBatch {
        indices: index_start..indices.len() as u32,
        clip,
    });
}

fn lyon_path(path: &PathData) -> lyon_path::Path {
    let mut builder = lyon_path::Path::builder().with_svg();
    for segment in path.segments() {
        match *segment {
            PathSegment::MoveTo(p) => {
                builder.move_to(point(p.x, p.y));
            }
            PathSegment::LineTo(p) => {
                builder.line_to(point(p.x, p.y));
            }
            PathSegment::QuadraticTo { control, to } => {
                builder.quadratic_bezier_to(point(control.x, control.y), point(to.x, to.y));
            }
            PathSegment::CubicTo {
                control1,
                control2,
                to,
            } => {
                builder.cubic_bezier_to(
                    point(control1.x, control1.y),
                    point(control2.x, control2.y),
                    point(to.x, to.y),
                );
            }
            PathSegment::Close => {
                builder.close();
            }
        }
    }
    builder.build()
}

fn tessellate_fill(path: &PathData, rule: FillRule) -> Mesh {
    let path = lyon_path(path);
    let mut geometry: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    let options = FillOptions::tolerance(0.1).with_fill_rule(match rule {
        FillRule::NonZero => lyon_tessellation::FillRule::NonZero,
        FillRule::EvenOdd => lyon_tessellation::FillRule::EvenOdd,
    });
    let _ = FillTessellator::new().tessellate_path(
        &path,
        &options,
        &mut BuffersBuilder::new(&mut geometry, |v: FillVertex| v.position().to_array()),
    );
    Mesh {
        vertices: geometry.vertices,
        indices: geometry.indices,
    }
}

fn tessellate_stroke(path: &PathData, stroke: PathStroke) -> Mesh {
    let path = lyon_path(path);
    let cap = match stroke.cap {
        LineCap::Butt => lyon_tessellation::LineCap::Butt,
        LineCap::Square => lyon_tessellation::LineCap::Square,
        LineCap::Round => lyon_tessellation::LineCap::Round,
    };
    let join = match stroke.join {
        LineJoin::Miter => lyon_tessellation::LineJoin::Miter,
        LineJoin::Bevel => lyon_tessellation::LineJoin::Bevel,
        LineJoin::Round => lyon_tessellation::LineJoin::Round,
    };
    let options = StrokeOptions::tolerance(0.1)
        .with_line_width(stroke.width)
        .with_line_cap(cap)
        .with_line_join(join);
    let mut geometry: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    let _ = StrokeTessellator::new().tessellate_path(
        &path,
        &options,
        &mut BuffersBuilder::new(&mut geometry, |v: StrokeVertex| v.position().to_array()),
    );
    Mesh {
        vertices: geometry.vertices,
        indices: geometry.indices,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xui_interface::{PathBuilder, Point};

    fn triangle() -> PathData {
        let mut path = PathBuilder::new();
        path.move_to(Point::new(0.0, 0.0))
            .line_to(Point::new(10.0, 0.0))
            .line_to(Point::new(5.0, 10.0))
            .close();
        path.build()
    }

    #[test]
    fn tessellates_fill_and_stroke() {
        let path = triangle();
        let fill = tessellate_fill(&path, FillRule::NonZero);
        let stroke = tessellate_stroke(
            &path,
            PathStroke::new(Color::BLACK, 2.0)
                .cap(LineCap::Round)
                .join(LineJoin::Bevel),
        );
        assert!(!fill.vertices.is_empty());
        assert!(!fill.indices.is_empty());
        assert!(!stroke.vertices.is_empty());
        assert!(!stroke.indices.is_empty());
    }

    #[test]
    fn transformed_mesh_keeps_clip_and_color_out_of_geometry() {
        let mesh = tessellate_fill(&triangle(), FillRule::EvenOdd);
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut batches = Vec::new();
        let clip = Rect::new(1.0, 2.0, 30.0, 40.0);
        append_mesh(
            &mesh,
            Affine::scale(2.0, 3.0).then(Affine::translate(4.0, 5.0)),
            Color::rgb(1.0, 0.0, 0.0),
            clip,
            &mut vertices,
            &mut indices,
            &mut batches,
        );
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].clip, clip);
        assert_eq!(vertices[0].color, [1.0, 0.0, 0.0, 1.0]);

        append_mesh(
            &mesh,
            Affine::translate(1000.0, 1000.0),
            Color::BLACK,
            clip,
            &mut vertices,
            &mut indices,
            &mut batches,
        );
        assert_eq!(batches.len(), 1, "offscreen geometry must be culled");
    }
}
