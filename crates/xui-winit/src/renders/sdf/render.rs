use wgpu::{include_wgsl, util::DeviceExt};

use crate::wgpu::{SCENE_FORMAT, SCENE_SAMPLE_COUNT};
use xui_interface::Rect;

const UI_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 10] = wgpu::vertex_attr_array![
    0 => Float32x4,
    1 => Float32x4,
    2 => Float32x4,
    3 => Float32x4,
    4 => Float32x4,
    5 => Float32x4,
    6 => Float32x4,
    7 => Float32x4,
    8 => Float32x4,
    9 => Float32x4,
];

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SdfInstance {
    pub bounds: [f32; 4],
    pub shape: [f32; 4],
    pub clip: [f32; 4],
    pub fill_color: [f32; 4],
    pub stroke_color: [f32; 4],
    pub params: [f32; 4],
    pub stroke_params: [f32; 4],
    pub projection_color: [f32; 4],
    pub projection_params: [f32; 4],
    pub extra: [f32; 4],
}

impl SdfInstance {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &UI_INSTANCE_ATTRIBUTES,
        }
    }
}

pub struct SdfRenderer {
    pipeline: wgpu::RenderPipeline,
    instance_buffer: wgpu::Buffer,
    buffer_size: usize,
    instance_count: u32,
}

impl SdfRenderer {
    pub fn new(device: &wgpu::Device, tool_layout: &wgpu::BindGroupLayout) -> Self {
        let shader = include_str!("../../shaders/ui.wgsl");

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xui sdf shader"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xui sdf pipeline layout"),
            bind_group_layouts: &[Some(tool_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xui sdf render pipeline"),
            layout: Some(&pipeline_layout),

            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[SdfInstance::layout()],
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

            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },

            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: SCENE_SAMPLE_COUNT,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });

        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("xui sdf instances"),
            size: 0,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            instance_buffer,
            buffer_size: 0,
            instance_count: 0,
        }
    }

    pub fn deal_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[SdfInstance],
    ) {
        if instances.is_empty() {
            return;
        }

        let buffer_size = instances.len() * std::mem::size_of::<SdfInstance>();
        if buffer_size > self.buffer_size {
            self.buffer_size = buffer_size;
            self.instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("xui sdf instances"),
                contents: bytemuck::cast_slice(instances),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });

            self.instance_count = instances.len() as u32;
            return;
        }

        self.instance_count = instances.len() as u32;
        queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(instances));
    }

    pub fn render(
        &self,
        pass: &mut wgpu::RenderPass,
        ui_bind_group: &wgpu::BindGroup,
        scissors: &[Rect],
        scale_factor: f32,
        target_size: (u32, u32),
    ) {
        self.render_range(
            pass,
            ui_bind_group,
            scissors,
            0..self.instance_count as usize,
            scale_factor,
            target_size,
        );
    }

    pub fn render_range(
        &self,
        pass: &mut wgpu::RenderPass,
        ui_bind_group: &wgpu::BindGroup,
        scissors: &[Rect],
        range: std::ops::Range<usize>,
        scale_factor: f32,
        target_size: (u32, u32),
    ) {
        if range.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, ui_bind_group, &[]);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        let mut start = range.start;
        while start < range.end {
            let scissor = scissors[start];
            let mut end = start + 1;
            while end < range.end && scissors[end] == scissor {
                end += 1;
            }
            if let Some((x, y, width, height)) =
                crate::wgpu::physical_scissor(scissor, scale_factor, target_size)
            {
                pass.set_scissor_rect(x, y, width, height);
                pass.draw(0..6, start as u32..end as u32);
            }
            start = end;
        }
    }
}
