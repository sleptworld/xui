use wgpu::{BindGroupLayout, util::DeviceExt};

use crate::{
    text::atlas::Atlas,
    wgpu::{PType, SCENE_FORMAT, TextGlyphRecord},
};

const GLYPH_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
    0 => Uint32,
    1 => Float32x4,
    2 => Float32,
    3 => Float32x3,
    4 => Float32x4,
    5 => Float32x4,
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

pub struct GlyphBuffer {
    buffer: wgpu::Buffer,
    size: u64,
    len: usize,
}

impl GlyphBuffer {}

pub struct GlyphRender {
    subpixel_pipelines: [wgpu::RenderPipeline; 3],
    mask_pipeline: wgpu::RenderPipeline,
    color_pipeline: wgpu::RenderPipeline,

    glyph_bind_group_layout: BindGroupLayout,
    glyph_bind_group: wgpu::BindGroup,

    // Instance Buffer
    buffer: wgpu::Buffer,
    buffer_size: u64,
    buffer_len: usize,
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
            "fs_main_mask",
        );

        let color_pipeline = create_mask_glyph_pipeline(
            &device,
            &glyph_pipeline_layout,
            &glyph_shader,
            "xui glyph color render pipeline",
            "fs_main_color",
        );

        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("xui glyph instance buffer"),
            size: (size_of::<GlyphInstance>() * 10000) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            subpixel_pipelines,
            mask_pipeline,
            color_pipeline,
            glyph_bind_group_layout,
            glyph_bind_group,
            buffer,

            buffer_size: 10000,
            buffer_len: 0,
        }
    }

    pub fn deal_glyphs(&mut self, glyphs: Vec<TextGlyphRecord>) {
        for glyph in glyphs {
            match glyph.ptype {
                PType::Mask => {}
                PType::SubPixelMask => {}
                PType::Color => {}
            }
        }
    }

    fn copy_to_buffer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        glyphs: Vec<GlyphInstance>,
    ) {
        if glyphs.is_empty() {
            return;
        }

        if glyphs.len() as u64 > self.buffer_size {
            let new_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("xui glyph instance buffer"),
                contents: bytemuck::cast_slice(&glyphs),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
            self.buffer = new_buffer;
            self.buffer_size = glyphs.len() as u64;
            self.buffer_len = glyphs.len();
        } else {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&glyphs));
            self.buffer_len = glyphs.len();
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
                blend: None,
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
