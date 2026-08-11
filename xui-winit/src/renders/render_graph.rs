//! WGPU executor for the backend-independent `xui-render-graph` pass IR.
//!
//! This module deliberately knows nothing about retained layer styles. Its only
//! scheduling input is a lowered [`LayerRenderPlan`].

use std::{collections::HashMap, sync::Arc};

use wgpu::util::DeviceExt;
use xui::render::render_graph::{BuiltLayerProgram, ImageResource};
use xui_interface::{Affine, ImageData, ImageDataId, ImageFormat, Rect};
use xui_render_graph::{
    AttachmentBlend, Axis, BlendMode, CompositeOperator, DrawShader, ExternalResourceKind,
    LayerRenderPlan, MaskShape, Pass, PassOp, PipelineKey, PlanMask, PlanResourceId,
    PlanResourceKind,
};

use crate::{
    renders::image::{CachedImageTexture, ImageRender},
    wgpu::{
        SCENE_FORMAT, SCENE_SAMPLE_COUNT, WgpuBackendError,
        texture_pool::{TextureLease, TexturePool, TextureRequest},
    },
};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PassUniform {
    op: [u32; 4],
    output: [f32; 4],
    input0: [f32; 4],
    input1: [f32; 4],
    input2: [f32; 4],
    params0: [f32; 4],
    params1: [f32; 4],
    color: [f32; 4],
    inverse0: [f32; 4],
    inverse1: [f32; 4],
    matrix: [[f32; 4]; 5],
}

impl Default for PassUniform {
    fn default() -> Self {
        Self {
            op: [0; 4],
            output: [0.0; 4],
            input0: [0.0; 4],
            input1: [0.0; 4],
            input2: [0.0; 4],
            params0: [0.0; 4],
            params1: [0.0; 4],
            color: [0.0; 4],
            inverse0: [1.0, 0.0, 0.0, 1.0],
            inverse1: [0.0; 4],
            matrix: identity_color_matrix(),
        }
    }
}

pub struct GraphTarget<'a> {
    pub view: &'a wgpu::TextureView,
    pub msaa_view: &'a wgpu::TextureView,
    pub extent: (u32, u32),
    /// Complete logical domain represented by `view`, not the graph's clip.
    pub logical_bounds: Rect,
}

pub struct GraphTexture<'a> {
    pub view: &'a wgpu::TextureView,
    pub extent: (u32, u32),
    pub logical_bounds: Rect,
}

struct OwnedTexture {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    extent: (u32, u32),
}

impl OwnedTexture {
    fn scene(device: &wgpu::Device, extent: (u32, u32), label: &str) -> Self {
        let extent = (extent.0.max(1), extent.1.max(1));
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: extent.0,
                height: extent.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SCENE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _texture: texture,
            view,
            extent,
        }
    }
}

struct MaskTexture {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    extent: (u32, u32),
}

pub struct RenderGraphRenderer {
    bind_group_layout: wgpu::BindGroupLayout,
    filter_pipeline: wgpu::RenderPipeline,
    composite_replace: wgpu::RenderPipeline,
    composite_src_over: wgpu::RenderPipeline,
    composite_dst_over: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    fallback: OwnedTexture,
    texture_pool: TexturePool,
    transient_slots: Vec<TextureLease>,
    masks: HashMap<ImageDataId, MaskTexture>,
}

impl RenderGraphRenderer {
    pub fn new(device: &wgpu::Device, texture_pool: TexturePool) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("xui render graph bind group layout"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
                texture_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
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
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("xui render graph linear sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let filter_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xui render graph filter shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/render_graph_filter.wgsl").into(),
            ),
        });
        let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xui render graph composite shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/render_graph_composite.wgsl").into(),
            ),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xui render graph pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let filter_pipeline = create_pipeline(
            device,
            &pipeline_layout,
            &filter_shader,
            "xui render graph filter pipeline",
            1,
            None,
        );
        let composite_replace = create_pipeline(
            device,
            &pipeline_layout,
            &composite_shader,
            "xui render graph replace composite pipeline",
            SCENE_SAMPLE_COUNT,
            None,
        );
        let composite_src_over = create_pipeline(
            device,
            &pipeline_layout,
            &composite_shader,
            "xui render graph source-over composite pipeline",
            SCENE_SAMPLE_COUNT,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        let dst_over = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let composite_dst_over = create_pipeline(
            device,
            &pipeline_layout,
            &composite_shader,
            "xui render graph destination-over composite pipeline",
            SCENE_SAMPLE_COUNT,
            Some(dst_over),
        );

        Self {
            bind_group_layout,
            filter_pipeline,
            composite_replace,
            composite_src_over,
            composite_dst_over,
            sampler,
            fallback: OwnedTexture::scene(device, (1, 1), "xui render graph fallback"),
            texture_pool,
            transient_slots: Vec::new(),
            masks: HashMap::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        plan: &LayerRenderPlan,
        program: &BuiltLayerProgram,
        target: GraphTarget<'_>,
        layer_content: GraphTexture<'_>,
        parent_destination: GraphTexture<'_>,
        backdrop: Option<GraphTexture<'_>>,
        image_render: &ImageRender,
        scale_factor: f32,
    ) -> Result<(), WgpuBackendError> {
        if target.extent.0 == 0
            || target.extent.1 == 0
            || target.logical_bounds.width < 0.0
            || target.logical_bounds.height < 0.0
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "render graph target has an invalid extent or logical domain",
            )
            .into());
        }
        let result = (|| {
            self.ensure_transients(device, plan)?;
            self.ensure_bound_masks(device, queue, program)?;
            let keyed_masks = collect_keyed_masks(program, image_render)?;

            for pass in plan.passes() {
                self.encode_pass(
                    device,
                    encoder,
                    plan,
                    pass,
                    program,
                    &target,
                    &layer_content,
                    &parent_destination,
                    backdrop.as_ref(),
                    &keyed_masks,
                    scale_factor,
                )?;
            }
            Ok(())
        })();
        // Recorded commands retain their resource handles. Returning these
        // leases now enables ordered reuse by the next graph in this encoder.
        self.transient_slots.clear();
        self.texture_pool.trim();
        result
    }

    fn ensure_transients(
        &mut self,
        device: &wgpu::Device,
        plan: &LayerRenderPlan,
    ) -> Result<(), WgpuBackendError> {
        for (index, slot) in plan.slots().iter().enumerate() {
            let required = (slot.extent.width.max(1), slot.extent.height.max(1));
            let compatible = self.transient_slots.get(index).is_some_and(|texture| {
                let extent = texture.allocation_extent();
                extent.width >= required.0 && extent.height >= required.1
            });
            if compatible {
                continue;
            }
            let replacement = self.texture_pool.acquire(
                device,
                TextureRequest::scene(required, "xui render graph transient texture slot"),
            )?;
            if index < self.transient_slots.len() {
                self.transient_slots[index] = replacement;
            } else {
                self.transient_slots.push(replacement);
            }
        }
        self.transient_slots.truncate(plan.slots().len());
        Ok(())
    }

    fn ensure_bound_masks(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        program: &BuiltLayerProgram,
    ) -> Result<(), WgpuBackendError> {
        for binding in program.bindings().layer_masks() {
            if let ImageResource::Data { data, .. } = &binding.handle {
                self.ensure_mask(device, queue, data)?;
            }
        }
        if let Some(ImageResource::Data { data, .. }) = program.bindings().backdrop_mask() {
            self.ensure_mask(device, queue, data)?;
        }
        Ok(())
    }

    fn ensure_mask(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        data: &ImageData,
    ) -> Result<(), WgpuBackendError> {
        if self.masks.contains_key(&data.id()) {
            return Ok(());
        }
        if data.size.width == 0
            || data.size.height == 0
            || data.pixels.len() != data.size.width as usize * data.size.height as usize * 4
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid render-graph mask image data",
            )
            .into());
        }
        let format = match data.format {
            ImageFormat::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8UnormSrgb,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("xui render graph mask"),
            size: wgpu::Extent3d {
                width: data.size.width,
                height: data.size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            data.pixels.as_ref(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(data.size.width * 4),
                rows_per_image: Some(data.size.height),
            },
            wgpu::Extent3d {
                width: data.size.width,
                height: data.size.height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.masks.insert(
            data.id(),
            MaskTexture {
                _texture: texture,
                view,
                extent: (data.size.width, data.size.height),
            },
        );
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_pass(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        plan: &LayerRenderPlan,
        pass: &Pass,
        program: &BuiltLayerProgram,
        target: &GraphTarget<'_>,
        layer_content: &GraphTexture<'_>,
        parent_destination: &GraphTexture<'_>,
        backdrop: Option<&GraphTexture<'_>>,
        keyed_masks: &[(ExternalResourceKind, Arc<CachedImageTexture>)],
        scale_factor: f32,
    ) -> Result<(), WgpuBackendError> {
        let output = self.resource(
            plan,
            pass.output,
            program,
            layer_content,
            parent_destination,
            backdrop,
            keyed_masks,
            scale_factor,
        )?;
        let input = |id: Option<PlanResourceId>| -> Result<TextureRef<'_>, WgpuBackendError> {
            id.map(|id| {
                self.resource(
                    plan,
                    id,
                    program,
                    layer_content,
                    parent_destination,
                    backdrop,
                    keyed_masks,
                    scale_factor,
                )
            })
            .transpose()
            .map(|value| value.unwrap_or(self.fallback_ref()))
        };
        let source0 = input(pass.bindings.texture0)?;
        let source1 = input(pass.bindings.texture1)?;
        let source2 = input(pass.bindings.texture2)?;
        let mut uniform = pass_uniform(pass, output, scale_factor);
        match pass.draw.shader {
            DrawShader::AttachmentBackdrop => uniform.op[0] = attachment_shader_mode(false),
            DrawShader::AttachmentLayer => uniform.op[0] = attachment_shader_mode(true),
            DrawShader::Filter | DrawShader::Composite => {}
        }
        uniform.input0 = source0.info(scale_factor);
        uniform.input1 = source1.info(scale_factor);
        uniform.input2 = source2.info(scale_factor);
        let bind_group = self.bind_group(device, &source0, &source1, &source2, &uniform);

        if let PipelineKey::Composite(blend) = pass.pipeline {
            let draw = pass.draw;
            if draw.scissor.width == 0 || draw.scissor.height == 0 {
                return Ok(());
            }
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xui render graph composite pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target.msaa_view,
                    resolve_target: Some(target.view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            render_pass.set_pipeline(match blend {
                AttachmentBlend::Replace => &self.composite_replace,
                AttachmentBlend::SrcOver => &self.composite_src_over,
                AttachmentBlend::DstOver => &self.composite_dst_over,
            });
            render_pass.set_viewport(
                draw.viewport.x.max(0) as f32,
                draw.viewport.y.max(0) as f32,
                draw.viewport.width as f32,
                draw.viewport.height as f32,
                0.0,
                1.0,
            );
            render_pass.set_scissor_rect(
                draw.scissor.x.max(0) as u32,
                draw.scissor.y.max(0) as u32,
                draw.scissor.width,
                draw.scissor.height,
            );
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.draw(0..draw.vertex_count, 0..1);
        } else {
            let draw = pass.draw;
            if draw.viewport.width == 0
                || draw.viewport.height == 0
                || draw.scissor.width == 0
                || draw.scissor.height == 0
            {
                return Ok(());
            }
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xui render graph filter pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: output.view,
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
            render_pass.set_pipeline(&self.filter_pipeline);
            render_pass.set_viewport(
                draw.viewport.x.max(0) as f32,
                draw.viewport.y.max(0) as f32,
                draw.viewport.width as f32,
                draw.viewport.height as f32,
                0.0,
                1.0,
            );
            render_pass.set_scissor_rect(
                draw.scissor.x.max(0) as u32,
                draw.scissor.y.max(0) as u32,
                draw.scissor.width,
                draw.scissor.height,
            );
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.draw(0..draw.vertex_count, 0..1);
        }
        Ok(())
    }

    fn bind_group(
        &self,
        device: &wgpu::Device,
        source0: &TextureRef<'_>,
        source1: &TextureRef<'_>,
        source2: &TextureRef<'_>,
        uniform: &PassUniform,
    ) -> wgpu::BindGroup {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("xui render graph pass uniform"),
            contents: bytemuck::bytes_of(uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xui render graph pass bind group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source0.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(source1.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(source2.view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buffer.as_entire_binding(),
                },
            ],
        })
    }

    fn fallback_ref(&self) -> TextureRef<'_> {
        TextureRef {
            view: &self.fallback.view,
            extent: self.fallback.extent,
            logical_bounds: Rect::ZERO,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn resource<'a>(
        &'a self,
        plan: &LayerRenderPlan,
        id: PlanResourceId,
        program: &BuiltLayerProgram,
        layer_content: &'a GraphTexture<'_>,
        parent_destination: &'a GraphTexture<'_>,
        backdrop: Option<&'a GraphTexture<'_>>,
        keyed_masks: &'a [(ExternalResourceKind, Arc<CachedImageTexture>)],
        scale_factor: f32,
    ) -> Result<TextureRef<'a>, WgpuBackendError> {
        let resource = &plan.resources()[id.index()];
        match resource.kind {
            PlanResourceKind::Transient => {
                let slot = resource
                    .slot
                    .expect("transient resources have allocated slots");
                let texture = &self.transient_slots[slot.index()];
                Ok(TextureRef {
                    view: texture.view(),
                    // A transient slot can be larger than the resource that
                    // currently occupies it. Keep the valid content extent
                    // separate from the allocation extent queried by WGSL.
                    extent: (
                        resource.physical_bounds.width,
                        resource.physical_bounds.height,
                    ),
                    logical_bounds: Rect::new(
                        resource.physical_bounds.x as f32 / scale_factor,
                        resource.physical_bounds.y as f32 / scale_factor,
                        resource.physical_bounds.width as f32 / scale_factor,
                        resource.physical_bounds.height as f32 / scale_factor,
                    ),
                })
            }
            PlanResourceKind::External(kind) => match kind {
                ExternalResourceKind::Backdrop => backdrop
                    .map(|texture| TextureRef {
                        view: texture.view,
                        extent: texture.extent,
                        logical_bounds: texture.logical_bounds,
                    })
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "render plan demands an independent backdrop texture",
                        )
                        .into()
                    }),
                ExternalResourceKind::ParentDestination => Ok(TextureRef {
                    view: parent_destination.view,
                    extent: parent_destination.extent,
                    logical_bounds: parent_destination.logical_bounds,
                }),
                ExternalResourceKind::LayerContent => Ok(TextureRef {
                    view: layer_content.view,
                    extent: layer_content.extent,
                    logical_bounds: layer_content.logical_bounds,
                }),
                ExternalResourceKind::BackdropMask | ExternalResourceKind::LayerMask(_) => {
                    self.mask_resource(kind, program, keyed_masks)
                }
            },
        }
    }

    fn mask_resource<'a>(
        &'a self,
        kind: ExternalResourceKind,
        program: &BuiltLayerProgram,
        keyed_masks: &'a [(ExternalResourceKind, Arc<CachedImageTexture>)],
    ) -> Result<TextureRef<'a>, WgpuBackendError> {
        let handle = program.handle(kind).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("missing render-graph binding for {kind:?}"),
            )
        })?;
        match handle {
            ImageResource::Data { data, .. } => {
                let texture = &self.masks[&data.id()];
                Ok(TextureRef {
                    view: &texture.view,
                    extent: texture.extent,
                    logical_bounds: Rect::new(0.0, 0.0, 1.0, 1.0),
                })
            }
            ImageResource::Key(_) => {
                let texture = keyed_masks
                    .iter()
                    .find(|(candidate, _)| *candidate == kind)
                    .map(|(_, texture)| texture)
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            format!("unresolved render-graph mask {kind:?}"),
                        )
                    })?;
                Ok(TextureRef {
                    view: &texture.view,
                    extent: texture.extent,
                    logical_bounds: Rect::new(0.0, 0.0, 1.0, 1.0),
                })
            }
        }
    }
}

#[derive(Clone, Copy)]
struct TextureRef<'a> {
    view: &'a wgpu::TextureView,
    extent: (u32, u32),
    logical_bounds: Rect,
}

impl TextureRef<'_> {
    fn info(self, _scale_factor: f32) -> [f32; 4] {
        // z/w describe valid content, which may be smaller than the texture
        // allocation when a render-graph transient slot is reused.
        [
            self.logical_bounds.x,
            self.logical_bounds.y,
            self.extent.0 as f32,
            self.extent.1 as f32,
        ]
    }
}

fn collect_keyed_masks(
    program: &BuiltLayerProgram,
    images: &ImageRender,
) -> Result<Vec<(ExternalResourceKind, Arc<CachedImageTexture>)>, WgpuBackendError> {
    let mut result = Vec::new();
    for resource in program.program().resources() {
        let xui_render_graph::ProgramResourceKind::External(kind) = resource.kind else {
            continue;
        };
        let Some(ImageResource::Key(key)) = program.handle(kind) else {
            continue;
        };
        let texture = images.cached_texture(key).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("image-backed render-graph mask {key:?} is not resident"),
            )
        })?;
        result.push((kind, texture));
    }
    Ok(result)
}

fn pass_uniform(pass: &Pass, output: TextureRef<'_>, scale_factor: f32) -> PassUniform {
    debug_assert_eq!(pass.uniforms.scale_factor.to_bits(), scale_factor.to_bits());
    let output_origin = output.logical_bounds;
    let mut value = PassUniform {
        output: [output_origin.x, output_origin.y, scale_factor, 0.0],
        ..PassUniform::default()
    };
    match &pass.op {
        PassOp::Copy => value.op[0] = 0,
        PassOp::GaussianBlur {
            axis,
            sigma_px,
            support,
        } => {
            value.op[0] = match axis {
                Axis::X => 1,
                Axis::Y => 2,
            };
            value.params0 = [*sigma_px, *support, 0.0, 0.0];
        }
        PassOp::ColorMatrix(matrix) => {
            value.op[0] = 3;
            value.matrix = matrix_to_rows(*matrix);
        }
        PassOp::Pixelate {
            block_width_px,
            block_height_px,
        } => {
            value.op[0] = 4;
            value.params0 = [*block_width_px, *block_height_px, 0.0, 0.0];
        }
        PassOp::Refraction {
            strength_px,
            chromatic_aberration_px,
        } => {
            value.op[0] = 5;
            value.params0 = [*strength_px, *chromatic_aberration_px, 0.0, 0.0];
            value.params1 = [
                pass.uniforms.output_logical_bounds.x
                    + pass.uniforms.output_logical_bounds.width * 0.5,
                pass.uniforms.output_logical_bounds.y
                    + pass.uniforms.output_logical_bounds.height * 0.5,
                0.0,
                0.0,
            ];
        }
        PassOp::ChromaticAberration { offset_px } => {
            value.op[0] = 6;
            value.params0 = [offset_px[0], offset_px[1], 0.0, 0.0];
        }
        PassOp::ExtractAlpha => value.op[0] = 7,
        PassOp::AlphaSpread { axis, radius_px } => {
            value.op[0] = if *axis == Axis::X { 8 } else { 9 };
            value.params0[0] = *radius_px;
        }
        PassOp::ShadowComposite { color, offset_px } => {
            value.op[0] = 10;
            value.params0 = [offset_px[0], offset_px[1], 0.0, 0.0];
            value.color = [color.r, color.g, color.b, color.a];
        }
        PassOp::ApplyMask { transform, .. } => {
            value.op[0] = 11;
            set_inverse(&mut value, *transform);
        }
        PassOp::BackdropComposite {
            opacity,
            blend_mode,
            mask,
            ..
        } => {
            value.op = [0, blend_index(*blend_mode), 0, mask_kind(mask)];
            value.params0[0] = *opacity;
            set_mask(&mut value, mask);
        }
        PassOp::LayerComposite {
            opacity,
            transform,
            blend_mode,
            operator,
            ..
        } => {
            value.op = [1, blend_index(*blend_mode), operator_index(*operator), 0];
            value.params0[0] = *opacity;
            set_inverse(&mut value, *transform);
        }
    }
    value
}

fn set_mask(uniform: &mut PassUniform, mask: &PlanMask) {
    match mask {
        PlanMask::None => {}
        PlanMask::Shape { shape, transform } => {
            set_inverse(uniform, *transform);
            match shape {
                MaskShape::RoundedRect(radius) => {
                    let x_scale = transform.xx.hypot(transform.yx);
                    let y_scale = transform.xy.hypot(transform.yy);
                    uniform.params1[0] = *radius / x_scale.min(y_scale).max(f32::EPSILON);
                }
                MaskShape::Line { from, to } => {
                    uniform.params1 = [from.x, from.y, 0.5, 0.0];
                    uniform.color = [to.x, to.y, 0.0, 0.0];
                }
                MaskShape::Rect | MaskShape::Circle | MaskShape::Ellipse => {}
            }
        }
        PlanMask::Texture { transform, .. } => set_inverse(uniform, *transform),
    }
}

fn set_inverse(uniform: &mut PassUniform, transform: Affine) {
    if let Some(inverse) = inverse_affine(transform) {
        uniform.inverse0 = [inverse.xx, inverse.yx, inverse.xy, inverse.yy];
        uniform.inverse1 = [inverse.dx, inverse.dy, 0.0, 0.0];
    }
}

fn inverse_affine(transform: Affine) -> Option<Affine> {
    let determinant = transform.xx * transform.yy - transform.xy * transform.yx;
    if determinant.abs() <= f32::EPSILON {
        return None;
    }
    let inverse = determinant.recip();
    let xx = transform.yy * inverse;
    let xy = -transform.xy * inverse;
    let yx = -transform.yx * inverse;
    let yy = transform.xx * inverse;
    Some(Affine::new(
        xx,
        yx,
        xy,
        yy,
        -(xx * transform.dx + xy * transform.dy),
        -(yx * transform.dx + yy * transform.dy),
    ))
}

const fn attachment_shader_mode(transformed_source: bool) -> u32 {
    if transformed_source { 3 } else { 2 }
}

fn mask_kind(mask: &PlanMask) -> u32 {
    match mask {
        PlanMask::None => 0,
        PlanMask::Shape {
            shape: MaskShape::Rect,
            ..
        } => 1,
        PlanMask::Shape {
            shape: MaskShape::RoundedRect(_),
            ..
        } => 2,
        PlanMask::Shape {
            shape: MaskShape::Circle,
            ..
        } => 3,
        PlanMask::Shape {
            shape: MaskShape::Ellipse,
            ..
        } => 4,
        PlanMask::Shape {
            shape: MaskShape::Line { .. },
            ..
        } => 5,
        PlanMask::Texture { .. } => 6,
    }
}

fn blend_index(mode: BlendMode) -> u32 {
    match mode {
        BlendMode::Normal => 0,
        BlendMode::Multiply => 1,
        BlendMode::Screen => 2,
        BlendMode::Overlay => 3,
        BlendMode::Darken => 4,
        BlendMode::Lighten => 5,
        BlendMode::ColorDodge => 6,
        BlendMode::ColorBurn => 7,
        BlendMode::HardLight => 8,
        BlendMode::SoftLight => 9,
        BlendMode::Difference => 10,
        BlendMode::Exclusion => 11,
        BlendMode::Hue => 12,
        BlendMode::Saturation => 13,
        BlendMode::Color => 14,
        BlendMode::Luminosity => 15,
    }
}

fn operator_index(operator: CompositeOperator) -> u32 {
    match operator {
        CompositeOperator::SrcOver => 0,
        CompositeOperator::Src => 1,
        CompositeOperator::DstOver => 2,
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn create_pipeline(
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
                format: SCENE_FORMAT,
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

const fn identity_color_matrix() -> [[f32; 4]; 5] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
        [0.0; 4],
    ]
}

fn matrix_to_rows(matrix: [f32; 20]) -> [[f32; 4]; 5] {
    [
        [matrix[0], matrix[1], matrix[2], matrix[3]],
        [matrix[5], matrix[6], matrix[7], matrix[8]],
        [matrix[10], matrix[11], matrix[12], matrix[13]],
        [matrix[15], matrix[16], matrix[17], matrix[18]],
        [matrix[4], matrix[9], matrix[14], matrix[19]],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_graph_shaders_parse_and_validate() {
        for source in [
            include_str!("../shaders/render_graph_filter.wgsl"),
            include_str!("../shaders/render_graph_composite.wgsl"),
        ] {
            let module = naga::front::wgsl::parse_str(source).expect("shader must parse as WGSL");
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .expect("shader must pass naga validation");
        }
    }

    #[test]
    fn blend_and_operator_indices_match_ir_order() {
        let modes = [
            BlendMode::Normal,
            BlendMode::Multiply,
            BlendMode::Screen,
            BlendMode::Overlay,
            BlendMode::Darken,
            BlendMode::Lighten,
            BlendMode::ColorDodge,
            BlendMode::ColorBurn,
            BlendMode::HardLight,
            BlendMode::SoftLight,
            BlendMode::Difference,
            BlendMode::Exclusion,
            BlendMode::Hue,
            BlendMode::Saturation,
            BlendMode::Color,
            BlendMode::Luminosity,
        ];
        assert_eq!(
            modes.map(blend_index),
            std::array::from_fn(|index| index as u32)
        );
        assert_eq!(
            [
                CompositeOperator::SrcOver,
                CompositeOperator::Src,
                CompositeOperator::DstOver,
            ]
            .map(operator_index),
            [0, 1, 2]
        );
    }

    #[test]
    fn attachment_composites_preserve_layer_transform_sampling() {
        assert_eq!(attachment_shader_mode(false), 2);
        assert_eq!(attachment_shader_mode(true), 3);
    }
}
