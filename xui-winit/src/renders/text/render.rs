use super::atlas::Atlas;
use crate::wgpu::SCENE_FORMAT;
use glam::{Vec2, Vec3};
use std::ops::Range;
use wgpu::{BindGroup, BindGroupLayout, util::DeviceExt, wgc::device};
use xui::{Color, Rect};
use xui_interface::RasterizedGlyphFormat;

const GLYPH_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x4,
    1 => Float32,
    2 => Float32x3,
    3 => Float32x4,
    4 => Float32x4,
];

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GlyphInstance {
    bounds: [f32; 4],
    layer: f32,
    padding: [f32; 3],
    uv: [f32; 4],
    color: [f32; 4],
}

impl GlyphInstance {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &GLYPH_INSTANCE_ATTRIBUTES,
        }
    }
}

struct GlyphBuffer {
    buffer: wgpu::Buffer,
    size: u64,
    len: usize,
    scissors: Vec<Rect>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextGlyphRecord {
    pub ptype: RasterizedGlyphFormat,
    pub screen_rect: Rect,
    pub clip: Rect,
    pub color: Color,
    pub atlas_origin: Vec2,
    pub atlas_layer: u32,
    pub atlas_size: Vec3,
    pub atlas_rect: Rect,
}

impl GlyphBuffer {
    fn new<S: Sized>(device: &wgpu::Device, size: u64) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("xui glyph instance buffer"),
            size: size * std::mem::size_of::<S>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            buffer,
            size,
            len: 0,
            scissors: Vec::new(),
        }
    }

    fn copy_to_buffer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        glyphs: Vec<(GlyphInstance, Rect)>,
    ) {
        if glyphs.is_empty() {
            self.len = 0;
            self.scissors.clear();
            return;
        }
        let (glyphs, scissors): (Vec<_>, Vec<_>) = glyphs.into_iter().unzip();

        if glyphs.len() as u64 > self.size {
            let new_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("xui glyph instance buffer"),
                contents: bytemuck::cast_slice(&glyphs),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
            self.buffer = new_buffer;
            self.size = glyphs.len() as u64;
            self.len = glyphs.len();
            self.scissors = scissors;
        } else {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&glyphs));
            self.len = glyphs.len();
            self.scissors = scissors;
        }
    }

    fn len(&self) -> usize {
        self.len
    }
}

pub struct GlyphRender {
    subpixel_pipelines: [wgpu::RenderPipeline; 3],
    mask_pipeline: wgpu::RenderPipeline,
    color_pipeline: wgpu::RenderPipeline,

    glyph_bind_group_layout: BindGroupLayout,
    glyph_bind_group: wgpu::BindGroup,

    // Instance Buffer
    subpixel_buffer: GlyphBuffer,
    mask_buffer: GlyphBuffer,
    color_buffer: GlyphBuffer,
    record_ranges: Vec<GlyphRecordRanges>,
    // Atlas
    atlas: Atlas,
}

#[derive(Clone, Default)]
struct GlyphRecordRanges {
    mask: Range<usize>,
    subpixel: Range<usize>,
    color: Range<usize>,
}

impl GlyphRender {
    pub fn new(device: &wgpu::Device, atlas: &Atlas, common_tools: &BindGroupLayout) -> Self {
        let glyph_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xui glyph shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/glyph.wgsl").into()),
        });

        let glyph_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("xui glyph atlas bind group layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D3,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                        count: None,
                    },
                ],
            });

        let glyph_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xui glyph atlas bind group"),
            layout: &glyph_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(atlas.view()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(atlas.sampler()),
                },
            ],
        });

        let glyph_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("xui glyph pipeline layout"),
                bind_group_layouts: &[Some(common_tools), Some(&glyph_bind_group_layout)],
                immediate_size: 0,
            });

        let subpixel_pipelines = [
            create_glyph_pipeline(
                &device,
                &glyph_pipeline_layout,
                &glyph_shader,
                SCENE_FORMAT,
                "xui glyph red render pipeline",
                "fs_main_red",
                wgpu::ColorWrites::RED,
            ),
            create_glyph_pipeline(
                &device,
                &glyph_pipeline_layout,
                &glyph_shader,
                SCENE_FORMAT,
                "xui glyph green render pipeline",
                "fs_main_green",
                wgpu::ColorWrites::GREEN,
            ),
            create_glyph_pipeline(
                &device,
                &glyph_pipeline_layout,
                &glyph_shader,
                SCENE_FORMAT,
                "xui glyph blue render pipeline",
                "fs_main_blue",
                wgpu::ColorWrites::BLUE,
            ),
        ];

        let mask_pipeline = create_mask_glyph_pipeline(
            &device,
            &glyph_pipeline_layout,
            &glyph_shader,
            "xui glyph mask render pipeline",
            "fs_mask",
        );

        let color_pipeline = create_mask_glyph_pipeline(
            &device,
            &glyph_pipeline_layout,
            &glyph_shader,
            "xui glyph color render pipeline",
            "fs_rgb",
        );

        let mask_buffer = GlyphBuffer::new::<GlyphInstance>(&device, 1000);
        let color_buffer = GlyphBuffer::new::<GlyphInstance>(&device, 1000);
        let subpixel_buffer = GlyphBuffer::new::<GlyphInstance>(&device, 1000);

        let atlas = Atlas::new(device);

        Self {
            subpixel_pipelines,
            mask_pipeline,
            color_pipeline,
            glyph_bind_group_layout,
            glyph_bind_group,
            subpixel_buffer,
            mask_buffer,
            color_buffer,
            record_ranges: Vec::new(),
            atlas,
        }
    }

    pub fn deal_glyphs(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        glyphs: &Vec<TextGlyphRecord>,
    ) {
        let mut subpixel_glyphs = Vec::new();
        let mut mask_glyphs = Vec::new();
        let mut color_glyphs = Vec::new();
        let mut record_ranges = Vec::with_capacity(glyphs.len());
        for glyph in glyphs {
            let before = [mask_glyphs.len(), subpixel_glyphs.len(), color_glyphs.len()];
            if glyph.screen_rect.width <= 0.0
                || glyph.screen_rect.height <= 0.0
                || glyph.clip.width <= 0.0
                || glyph.clip.height <= 0.0
                || glyph.color.a <= 0.0
                || glyph.atlas_size.x <= 0.0
                || glyph.atlas_size.y <= 0.0
                || glyph.atlas_size.z <= 0.0
            {
                record_ranges.push(GlyphRecordRanges {
                    mask: before[0]..before[0],
                    subpixel: before[1]..before[1],
                    color: before[2]..before[2],
                });
                continue;
            }

            let glyph_instant = GlyphInstance {
                bounds: rect_to_array(glyph.screen_rect),
                layer: glyph.atlas_layer as f32 / glyph.atlas_size.z,
                padding: [0.0; 3],
                uv: [
                    glyph.atlas_rect.x / glyph.atlas_size.x,
                    glyph.atlas_rect.y / glyph.atlas_size.y,
                    glyph.atlas_rect.width / glyph.atlas_size.x,
                    glyph.atlas_rect.height / glyph.atlas_size.y,
                ],
                color: color_to_array(glyph.color),
            };
            match glyph.ptype {
                RasterizedGlyphFormat::Mask => {
                    mask_glyphs.push((glyph_instant, glyph.clip));
                }
                RasterizedGlyphFormat::SubpixelMask => {
                    subpixel_glyphs.push((glyph_instant, glyph.clip));
                }
                RasterizedGlyphFormat::Color => {
                    color_glyphs.push((glyph_instant, glyph.clip));
                }
            }
            record_ranges.push(GlyphRecordRanges {
                mask: before[0]..mask_glyphs.len(),
                subpixel: before[1]..subpixel_glyphs.len(),
                color: before[2]..color_glyphs.len(),
            });
        }
        self.record_ranges = record_ranges;

        self.mask_buffer.copy_to_buffer(device, queue, mask_glyphs);
        self.color_buffer
            .copy_to_buffer(device, queue, color_glyphs);
        self.subpixel_buffer
            .copy_to_buffer(device, queue, subpixel_glyphs);
    }

    pub fn render(
        &self,
        pass: &mut wgpu::RenderPass,
        tool_bind_group: &BindGroup,
        scale_factor: f32,
        target_size: (u32, u32),
    ) {
        self.render_range(
            pass,
            tool_bind_group,
            0..self.record_ranges.len(),
            scale_factor,
            target_size,
        );
    }

    pub fn render_range(
        &self,
        pass: &mut wgpu::RenderPass,
        tool_bind_group: &BindGroup,
        records: Range<usize>,
        scale_factor: f32,
        target_size: (u32, u32),
    ) {
        let mask = self.buffer_range(records.clone(), |ranges| &ranges.mask);
        let subpixel = self.buffer_range(records.clone(), |ranges| &ranges.subpixel);
        let color = self.buffer_range(records, |ranges| &ranges.color);
        if !mask.is_empty() {
            pass.set_pipeline(&self.mask_pipeline);
            pass.set_bind_group(0, tool_bind_group, &[]);
            pass.set_bind_group(1, &self.glyph_bind_group, &[]);
            pass.set_vertex_buffer(0, self.mask_buffer.buffer.slice(..));
            draw_glyph_buffer(pass, &self.mask_buffer, mask, scale_factor, target_size);
        }

        if !subpixel.is_empty() {
            for c in &self.subpixel_pipelines {
                pass.set_pipeline(c);
                pass.set_bind_group(0, tool_bind_group, &[]);
                pass.set_bind_group(1, &self.glyph_bind_group, &[]);
                pass.set_vertex_buffer(0, self.subpixel_buffer.buffer.slice(..));
                draw_glyph_buffer(
                    pass,
                    &self.subpixel_buffer,
                    subpixel.clone(),
                    scale_factor,
                    target_size,
                );
            }
        }

        if !color.is_empty() {
            pass.set_pipeline(&self.color_pipeline);
            pass.set_bind_group(0, tool_bind_group, &[]);
            pass.set_bind_group(1, &self.glyph_bind_group, &[]);
            pass.set_vertex_buffer(0, self.color_buffer.buffer.slice(..));
            draw_glyph_buffer(pass, &self.color_buffer, color, scale_factor, target_size);
        }
    }

    fn buffer_range(
        &self,
        records: Range<usize>,
        select: impl Fn(&GlyphRecordRanges) -> &Range<usize>,
    ) -> Range<usize> {
        let start = records
            .clone()
            .next()
            .map(|i| select(&self.record_ranges[i]).start)
            .unwrap_or(0);
        let end = records
            .last()
            .map(|i| select(&self.record_ranges[i]).end)
            .unwrap_or(start);
        start..end
    }
}

fn draw_glyph_buffer(
    pass: &mut wgpu::RenderPass,
    buffer: &GlyphBuffer,
    range: Range<usize>,
    scale_factor: f32,
    target_size: (u32, u32),
) {
    let mut start = range.start;
    while start < range.end {
        let scissor = buffer.scissors[start];
        let mut end = start + 1;
        while end < range.end && buffer.scissors[end] == scissor {
            end += 1;
        }
        if let Some((x, y, width, height)) =
            crate::wgpu::physical_scissor(scissor, scale_factor, target_size)
        {
            pass.set_scissor_rect(x, y, width, height);
            pass.draw(0..4, start as u32..end as u32);
        }
        start = end;
    }
}

fn create_glyph_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    label: &'static str,
    fragment_entry: &'static str,
    write_mask: wgpu::ColorWrites,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),

        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[GlyphInstance::layout()],
        },

        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::SrcAlpha,
                        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent::REPLACE,
                }),
                write_mask,
            })],
        }),

        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },

        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: crate::wgpu::SCENE_SAMPLE_COUNT,
            ..Default::default()
        },
        multiview_mask: None,
        cache: None,
    })
}

fn create_mask_glyph_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    label: &'static str,
    fragment_entry: &'static str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),

        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[GlyphInstance::layout()],
        },

        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: SCENE_FORMAT,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),

        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },

        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: crate::wgpu::SCENE_SAMPLE_COUNT,
            ..Default::default()
        },
        multiview_mask: None,
        cache: None,
    })
}

fn rect_to_array(rect: Rect) -> [f32; 4] {
    [rect.x, rect.y, rect.width, rect.height]
}

fn color_to_array(color: Color) -> [f32; 4] {
    [color.r, color.g, color.b, color.a]
}
