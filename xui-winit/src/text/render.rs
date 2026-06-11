use wgpu::{BindGroup, BindGroupLayout, util::DeviceExt, wgc::device};
use xui::{Color, Rect};
use xui_interface::widget::PType;

use crate::{
    text::atlas::Atlas,
    wgpu::{ SCENE_FORMAT, TextGlyphRecord},
};

const GLYPH_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x4,
    1 => Float32,
    2 => Float32x3,
    3 => Float32x4,
    4 => Float32x4,
];

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GlyphInstance {
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
        }
    }

    fn copy_to_buffer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        glyphs: Vec<GlyphInstance>,
    ) {
        if glyphs.is_empty() {
            self.len = 0;
            return;
        }

        if glyphs.len() as u64 > self.size {
            let new_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("xui glyph instance buffer"),
                contents: bytemuck::cast_slice(&glyphs),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
            self.buffer = new_buffer;
            self.size = glyphs.len() as u64;
            self.len = glyphs.len();
        } else {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&glyphs));
            self.len = glyphs.len();
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
}

impl GlyphRender {
    pub fn new(device: &wgpu::Device, atlas: &Atlas, common_tools: &BindGroupLayout) -> Self {
        let glyph_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xui glyph shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/glyph.wgsl").into()),
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

        Self {
            subpixel_pipelines,
            mask_pipeline,
            color_pipeline,
            glyph_bind_group_layout,
            glyph_bind_group,
            subpixel_buffer,
            mask_buffer,
            color_buffer,
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
        for glyph in glyphs {
            if glyph.screen_rect.width <= 0.0
                || glyph.screen_rect.height <= 0.0
                || glyph.clip.width <= 0.0
                || glyph.clip.height <= 0.0
                || glyph.color.a <= 0.0
                || glyph.atlas_size.x <= 0.0
                || glyph.atlas_size.y <= 0.0
                || glyph.atlas_size.z <= 0.0
            {
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
                PType::Mask => {
                    mask_glyphs.push(glyph_instant);
                }
                PType::SubPixelMask => {
                    subpixel_glyphs.push(glyph_instant);
                }
                PType::Color => {
                    color_glyphs.push(glyph_instant);
                }
            }
        }

        self.mask_buffer.copy_to_buffer(device, queue, mask_glyphs);
        self.color_buffer
            .copy_to_buffer(device, queue, color_glyphs);
        self.subpixel_buffer
            .copy_to_buffer(device, queue, subpixel_glyphs);
    }

    pub fn render(&self, pass: &mut wgpu::RenderPass, tool_bind_group: &BindGroup) {
        if self.mask_buffer.len() > 0 {
            pass.set_pipeline(&self.mask_pipeline);
            pass.set_bind_group(0, tool_bind_group, &[]);
            pass.set_bind_group(1, &self.glyph_bind_group, &[]);
            pass.set_vertex_buffer(0, self.mask_buffer.buffer.slice(..));
            pass.draw(0..4, 0..self.mask_buffer.len() as u32);
        }

        if self.subpixel_buffer.len() > 0 {
            for c in &self.subpixel_pipelines {
                pass.set_pipeline(c);
                pass.set_bind_group(0, tool_bind_group, &[]);
                pass.set_bind_group(1, &self.glyph_bind_group, &[]);
                pass.set_vertex_buffer(0, self.subpixel_buffer.buffer.slice(..));
                pass.draw(0..4, 0..self.subpixel_buffer.len() as u32);
            }
        }

        if self.color_buffer.len() > 0 {
            pass.set_pipeline(&self.color_pipeline);
            pass.set_bind_group(0, tool_bind_group, &[]);
            pass.set_bind_group(1, &self.glyph_bind_group, &[]);
            pass.set_vertex_buffer(0, self.color_buffer.buffer.slice(..));
            pass.draw(0..4, 0..self.color_buffer.len() as u32);
        }
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
        multisample: wgpu::MultisampleState::default(),
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
        multisample: wgpu::MultisampleState::default(),
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
