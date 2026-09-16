use wgpu::util::DeviceExt;
use xui_interface::{Affine, Rect};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TileCompositeUniform {
    frame_extent: [f32; 2],
    tile_origin: [f32; 2],
    valid_extent: [f32; 2],
    allocation_extent: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SceneBlitUniform {
    target: [f32; 4],
    source: [f32; 4],
    allocation: [f32; 4],
    map0: [f32; 4],
    map1: [f32; 4],
}

#[derive(Clone, Copy)]
pub struct CompositeTile<'a> {
    pub view: &'a wgpu::TextureView,
    pub origin: (i32, i32),
    pub valid_extent: (u32, u32),
    pub allocation_extent: (u32, u32),
}

#[derive(Clone, Copy)]
pub struct SceneBlitSource<'a> {
    pub view: &'a wgpu::TextureView,
    pub allocation_extent: (u32, u32),
    pub logical_bounds: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneBlitBlend {
    Replace,
    SrcOver,
}

pub struct Compositor {
    pipeline: wgpu::RenderPipeline,
    restore_pipeline: wgpu::RenderPipeline,
    scene_blit_replace: wgpu::RenderPipeline,
    scene_blit_src_over: wgpu::RenderPipeline,
    scene_blit_msaa_replace: wgpu::RenderPipeline,
    scene_blit_layout: wgpu::BindGroupLayout,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl Compositor {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("xui tiled compositor bind group layout"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("xui tiled compositor sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xui tiled compositor pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xui tiled compositor shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/composite.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xui tiled compositor pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let restore_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xui shared tile restore pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: crate::wgpu::SCENE_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: crate::wgpu::SCENE_SAMPLE_COUNT,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });
        let scene_blit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("xui scene blit bind group layout"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let scene_blit_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("xui scene blit pipeline layout"),
                bind_group_layouts: &[Some(&scene_blit_layout)],
                immediate_size: 0,
            });
        let scene_blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xui scene blit shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/scene_blit.wgsl").into()),
        });
        let scene_blit_replace = create_scene_blit_pipeline(
            device,
            &scene_blit_pipeline_layout,
            &scene_blit_shader,
            "xui scene blit replace pipeline",
            1,
            None,
        );
        let scene_blit_src_over = create_scene_blit_pipeline(
            device,
            &scene_blit_pipeline_layout,
            &scene_blit_shader,
            "xui scene blit source-over pipeline",
            1,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        let scene_blit_msaa_replace = create_scene_blit_pipeline(
            device,
            &scene_blit_pipeline_layout,
            &scene_blit_shader,
            "xui scene blit msaa replace pipeline",
            crate::wgpu::SCENE_SAMPLE_COUNT,
            None,
        );
        Self {
            pipeline,
            restore_pipeline,
            scene_blit_replace,
            scene_blit_src_over,
            scene_blit_msaa_replace,
            scene_blit_layout,
            bind_group_layout,
            sampler,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn blit_scene(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
        target_extent: (u32, u32),
        target_logical_bounds: Rect,
        source: SceneBlitSource<'_>,
        target_to_source: Affine,
        sample_count: u32,
        blend: SceneBlitBlend,
        clear: bool,
    ) {
        if target_extent.0 == 0 || target_extent.1 == 0 {
            return;
        }
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("xui scene blit uniform"),
            contents: bytemuck::bytes_of(&SceneBlitUniform {
                target: [
                    target_logical_bounds.x,
                    target_logical_bounds.y,
                    target_logical_bounds.width,
                    target_logical_bounds.height,
                ],
                source: [
                    source.logical_bounds.x,
                    source.logical_bounds.y,
                    source.logical_bounds.width,
                    source.logical_bounds.height,
                ],
                allocation: [
                    source.allocation_extent.0 as f32,
                    source.allocation_extent.1 as f32,
                    target_extent.0 as f32,
                    target_extent.1 as f32,
                ],
                map0: [
                    target_to_source.xx,
                    target_to_source.yx,
                    target_to_source.xy,
                    target_to_source.yy,
                ],
                map1: [target_to_source.dx, target_to_source.dy, 0.0, 0.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xui scene blit bind group"),
            layout: &self.scene_blit_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        let pipeline = if sample_count == crate::wgpu::SCENE_SAMPLE_COUNT {
            debug_assert_eq!(blend, SceneBlitBlend::Replace);
            &self.scene_blit_msaa_replace
        } else {
            debug_assert_eq!(sample_count, 1);
            match blend {
                SceneBlitBlend::Replace => &self.scene_blit_replace,
                SceneBlitBlend::SrcOver => &self.scene_blit_src_over,
            }
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("xui scene blit pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_viewport(
            0.0,
            0.0,
            target_extent.0 as f32,
            target_extent.1 as f32,
            0.0,
            1.0,
        );
        pass.set_scissor_rect(0, 0, target_extent.0, target_extent.1);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Restores a resolved persistent tile into the one shared multisampled
    /// attachment before a serial tile job resumes after another tile used it.
    pub fn restore_tile_msaa(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        msaa_view: &wgpu::TextureView,
        msaa_extent: (u32, u32),
        tile: CompositeTile<'_>,
    ) {
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("xui shared tile restore uniform"),
            contents: bytemuck::bytes_of(&TileCompositeUniform {
                frame_extent: [msaa_extent.0 as f32, msaa_extent.1 as f32],
                tile_origin: [0.0, 0.0],
                valid_extent: [tile.valid_extent.0 as f32, tile.valid_extent.1 as f32],
                allocation_extent: [
                    tile.allocation_extent.0 as f32,
                    tile.allocation_extent.1 as f32,
                ],
            }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xui shared tile restore bind group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(tile.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("xui shared tile restore pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: msaa_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.restore_pipeline);
        pass.set_scissor_rect(0, 0, tile.valid_extent.0, tile.valid_extent.1);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    pub fn composite_tiles(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        frame_extent: (u32, u32),
        tiles: &[CompositeTile<'_>],
    ) {
        let resources: Vec<_> = tiles
            .iter()
            .map(|tile| {
                let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("xui tiled compositor uniform"),
                    contents: bytemuck::bytes_of(&TileCompositeUniform {
                        frame_extent: [frame_extent.0 as f32, frame_extent.1 as f32],
                        tile_origin: [tile.origin.0 as f32, tile.origin.1 as f32],
                        valid_extent: [tile.valid_extent.0 as f32, tile.valid_extent.1 as f32],
                        allocation_extent: [
                            tile.allocation_extent.0 as f32,
                            tile.allocation_extent.1 as f32,
                        ],
                    }),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("xui tiled compositor bind group"),
                    layout: &self.bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(tile.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: uniform.as_entire_binding(),
                        },
                    ],
                });
                (uniform, bind_group)
            })
            .collect();

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("xui tiled frame composite pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.08,
                        g: 0.09,
                        b: 0.11,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        for (tile, (_, bind_group)) in tiles.iter().zip(&resources) {
            let left = tile.origin.0.max(0) as u32;
            let top = tile.origin.1.max(0) as u32;
            let right = (i64::from(tile.origin.0) + i64::from(tile.valid_extent.0))
                .clamp(0, i64::from(frame_extent.0)) as u32;
            let bottom = (i64::from(tile.origin.1) + i64::from(tile.valid_extent.1))
                .clamp(0, i64::from(frame_extent.1)) as u32;
            if right <= left || bottom <= top {
                continue;
            }
            pass.set_scissor_rect(left, top, right - left, bottom - top);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

fn create_scene_blit_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    label: &str,
    sample_count: u32,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: crate::wgpu::SCENE_FORMAT,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: sample_count,
            ..Default::default()
        },
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn scene_blit_shader_parses_and_validates() {
        let module = naga::front::wgsl::parse_str(include_str!("../shaders/scene_blit.wgsl"))
            .expect("scene blit shader must parse");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("scene blit shader must validate");
    }
}
