//! The compositor: every pass that is not 2D drawing.
//!
//! Vello rasterizes vectors and composes nothing. What it draws arrives here
//! as textures, and everything done to them is a full-screen pass of
//! [`filter.wgsl`](../filter.wgsl): bands of 2D content drawn over one another,
//! application textures drawn between them, and isolated layers run through
//! their render-graph plans (`graph`): filters, backdrops, masks, and the
//! composite into the parent. Everything is computed on sRGB-encoded values,
//! premultiplied: the space Vello blends in inside a band, so joining bands,
//! drawing text, compositing layers and filtering all agree with it (see the
//! crate's "Colour space").

use moka::sync::Cache;
use rustc_hash::FxBuildHasher;
use wgpu::util::DeviceExt;
use xui_interface::{ImageData, ImageDataId};
mod graph;

pub(crate) use graph::{Placed, PlanTextures};

/// What Vello writes and what the frame holds: sRGB-encoded values, 8 bits.
pub(crate) const ENCODED_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// What every filter and composite computes in: the same sRGB-encoded values,
/// premultiplied, in half-float so a chain of passes (a blur cut into stages,
/// a backdrop under a backdrop) does not round to 8 bits at every step.
pub(crate) const WORK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Which of the two a pooled texture holds. Both hold sRGB-encoded values;
/// they differ in alpha and in precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Space {
    /// 8 bits, straight alpha, as Vello writes it. Can be a Vello target.
    Encoded,
    /// Half-float, premultiplied. Where the work is done.
    Work,
}

impl Space {
    fn format(self) -> wgpu::TextureFormat {
        match self {
            Self::Encoded => ENCODED_FORMAT,
            Self::Work => WORK_FORMAT,
        }
    }

    fn bytes_per_texel(self) -> u64 {
        match self {
            Self::Encoded => 4,
            Self::Work => 8,
        }
    }
}

/// Whether sampling a texture of `format` gives linear light rather than the
/// sRGB-encoded values the compositor works in: an sRGB format, which the
/// hardware decodes, or a float one, which by convention holds linear light.
pub(crate) fn samples_linear(format: wgpu::TextureFormat) -> bool {
    format.is_srgb()
        || matches!(
            format,
            wgpu::TextureFormat::Rgba16Float
                | wgpu::TextureFormat::Rgba32Float
                | wgpu::TextureFormat::Rg11b10Ufloat
        )
}

/// A clip on an external texture, evaluated per pixel in the shader.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ClipSpec {
    /// Target pixels -> the clip's own space.
    pub inverse: [f32; 6],
    pub rect: [f32; 4],
    /// Zero for a plain rectangle.
    pub radius: f32,
}

/// A GPU texture the application rendered, drawn over what is composed so far.
///
/// Composed by this crate, not by Vello: the texture is sampled where it is,
/// with nothing copied.
#[derive(Clone)]
pub(crate) struct TextureDraw {
    pub texture: wgpu::Texture,
    /// Target pixels -> the texture's bounds space.
    pub inverse: [f32; 6],
    /// Where the texture is placed, in its own space: `[x0, y0, x1, y1]`.
    pub bounds: [f32; 4],
    pub opacity: f32,
    pub nearest: bool,
    /// At most four: a deeper chain is not representable.
    pub clips: Vec<ClipSpec>,
}

/// Where a composed target ends up.
pub(crate) enum Finish<'a> {
    Slot(Slot),
    View(&'a wgpu::TextureView),
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    op: [u32; 4],
    p0: [f32; 4],
    p1: [f32; 4],
    p2: [f32; 4],
    p3: [f32; 4],
    color: [f32; 4],
    inv0: [f32; 4],
    inv1: [f32; 4],
    m: [[f32; 4]; 5],
    tex0: [f32; 4],
    tex1: [f32; 4],
    tex2: [f32; 4],
    clips: [[f32; 4]; 16],
    origin: [i32; 4],
    inputs: [[i32; 4]; 3],
    mask_rect: [i32; 4],
}

/// An input read whole, aligned with the output: what every pass outside a
/// render plan does.
const WHOLE_INPUT: [i32; 4] = [0, 0, i32::MAX, i32::MAX];

impl Params {
    fn op(op: u32) -> Self {
        Self {
            op: [op, 0, 0, 0],
            p0: [0.0; 4],
            p1: [0.0; 4],
            p2: [0.0; 4],
            p3: [0.0; 4],
            color: [0.0; 4],
            inv0: [1.0, 0.0, 0.0, 1.0],
            inv1: [0.0; 4],
            m: [[0.0; 4]; 5],
            tex0: [0.0; 4],
            tex1: [0.0; 4],
            tex2: [0.0; 4],
            clips: [[0.0; 4]; 16],
            origin: [0; 4],
            inputs: [WHOLE_INPUT; 3],
            mask_rect: [0; 4],
        }
    }
}

/// How a pass's output is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Blend {
    /// Cleared, then written.
    Replace,
    /// Kept, and the result drawn over it (premultiplied source over).
    Over,
    /// Kept, and the result drawn under it (destination over).
    Under,
    /// Kept, and the result written where the scissor allows.
    Overwrite,
}

/// Where one pass draws.
struct Target<'a> {
    view: &'a wgpu::TextureView,
    space: Space,
    blend: Blend,
    /// `[x, y, width, height]` in the output's texels; the whole output if
    /// `None`.
    scissor: Option<[u32; 4]>,
}

struct Pooled {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    extent: (u32, u32),
    space: Space,
    busy: bool,
    /// Held across frames (a cached layer's content): never handed out, never
    /// trimmed, and outside the pool's budget.
    pinned: bool,
    /// The frame this texture was last handed out in.
    last_used: u64,
}

/// Pooled textures. A [`Slot`] is an index that stays valid for as long as
/// its texture is held: trimming leaves a hole rather than moving anything,
/// so a pinned texture keeps its slot across frames.
#[derive(Default)]
struct Pool {
    entries: Vec<Option<Pooled>>,
}

impl Pool {
    fn iter(&self) -> impl Iterator<Item = &Pooled> {
        self.entries.iter().flatten()
    }

    fn insert(&mut self, entry: Pooled) -> usize {
        match self.entries.iter().position(Option::is_none) {
            Some(index) => {
                self.entries[index] = Some(entry);
                index
            }
            None => {
                self.entries.push(Some(entry));
                self.entries.len() - 1
            }
        }
    }
}

impl std::ops::Index<usize> for Pool {
    type Output = Pooled;
    fn index(&self, index: usize) -> &Pooled {
        self.entries[index].as_ref().expect("a slot that is held")
    }
}

impl std::ops::IndexMut<usize> for Pool {
    fn index_mut(&mut self, index: usize) -> &mut Pooled {
        self.entries[index].as_mut().expect("a slot that is held")
    }
}

impl Pooled {
    fn bytes(&self) -> u64 {
        self.extent.0 as u64 * self.extent.1 as u64 * self.space.bytes_per_texel()
    }
}

/// A texture nobody has asked for in this many frames is dropped. Long enough
/// to ride out a layer that flickers, short enough that the sizes a window
/// passes through while it is being resized do not pile up.
const POOL_IDLE_FRAMES: u64 = 8;

/// Bytes of pooled filter textures kept between frames.
pub(crate) const DEFAULT_POOL_BUDGET: u64 = 256 * 1024 * 1024;

/// Bytes of mask images kept on the GPU.
const MASK_BUDGET: u64 = 32 * 1024 * 1024;

#[derive(Clone)]
struct Mask {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    bytes: u32,
}

pub(crate) struct FilterRenderer {
    /// One pipeline per kind of output: the shader is the same.
    pipeline_work: wgpu::RenderPipeline,
    pipeline_encoded: wgpu::RenderPipeline,
    /// Work space, drawn over what the output holds: a composite into a parent.
    pipeline_over: wgpu::RenderPipeline,
    /// Work space, drawn under what the output holds.
    pipeline_under: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    dummy: wgpu::TextureView,
    _dummy_texture: wgpu::Texture,
    pool: Pool,
    pool_budget: u64,
    frame: u64,
    masks: Cache<ImageDataId, Mask, FxBuildHasher>,
}

/// A texture checked out of the pool; give it back with [`FilterRenderer::release`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Slot(usize);

impl FilterRenderer {
    pub(crate) fn new(device: &wgpu::Device, pool_budget: u64) -> Self {
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("xui-vello filter layout"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture_entry(4),
                texture_entry(5),
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xui-vello filter"),
            source: wgpu::ShaderSource::Wgsl(include_str!("filter.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xui-vello filter"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make_pipeline = |format: wgpu::TextureFormat, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xui-vello filter"),
            layout: Some(&pipeline_layout),
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
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
            })
        };
        let pipeline_work = make_pipeline(WORK_FORMAT, None);
        let pipeline_encoded = make_pipeline(ENCODED_FORMAT, None);
        let pipeline_over = make_pipeline(
            WORK_FORMAT,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        let under = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let pipeline_under = make_pipeline(
            WORK_FORMAT,
            Some(wgpu::BlendState {
                color: under,
                alpha: under,
            }),
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("xui-vello filter sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let dummy_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("xui-vello filter dummy"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: ENCODED_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let dummy = dummy_texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            pipeline_work,
            pipeline_encoded,
            pipeline_over,
            pipeline_under,
            layout,
            sampler,
            dummy,
            _dummy_texture: dummy_texture,
            pool: Pool::default(),
            pool_budget,
            frame: 0,
            masks: Cache::builder()
                .max_capacity(MASK_BUDGET)
                .weigher(|_: &ImageDataId, mask: &Mask| mask.bytes.max(1))
                .build_with_hasher(FxBuildHasher),
        }
    }

    /// Every texture but the pinned is free again; call once per frame.
    ///
    /// This is also where the pool is trimmed: textures idle for
    /// [`POOL_IDLE_FRAMES`] go, and then the least recently used until the
    /// byte budget holds. Nothing unpinned is checked out here, and a pinned
    /// texture is never trimmed, so no held [`Slot`] is invalidated.
    pub(crate) fn begin_frame(&mut self) {
        self.frame += 1;
        let frame = self.frame;
        for entry in &mut self.pool.entries {
            let Some(texture) = entry else { continue };
            if texture.pinned {
                continue;
            }
            texture.busy = false;
            if frame.saturating_sub(texture.last_used) > POOL_IDLE_FRAMES {
                *entry = None;
            }
        }
        let (_, mut bytes) = self.pool_size();
        while bytes > self.pool_budget {
            let Some(oldest) = self
                .pool
                .entries
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| Some((index, entry.as_ref()?)))
                .filter(|(_, entry)| !entry.pinned)
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(index, _)| index)
            else {
                break;
            };
            bytes -= self.pool.entries[oldest].take().map_or(0, |entry| entry.bytes());
        }
        self.masks.run_pending_tasks();
    }

    /// Textures pooled for passes (pinned ones are the layer cache's), and
    /// their size in bytes.
    pub(crate) fn pool_size(&self) -> (usize, u64) {
        let free = || self.pool.iter().filter(|entry| !entry.pinned);
        (free().count(), free().map(Pooled::bytes).sum())
    }

    /// Keeps `slot` across frames, until [`Self::unpin`]. It stays checked out.
    pub(crate) fn pin(&mut self, slot: Slot) {
        let entry = &mut self.pool[slot.0];
        entry.pinned = true;
        entry.busy = true;
    }

    /// Gives a pinned texture back to the pool, to be reused or trimmed.
    pub(crate) fn unpin(&mut self, slot: Slot) {
        let frame = self.frame;
        let entry = &mut self.pool[slot.0];
        entry.pinned = false;
        entry.busy = false;
        entry.last_used = frame;
    }

    /// The bytes of a texture.
    pub(crate) fn bytes(&self, slot: Slot) -> u64 {
        self.pool[slot.0].bytes()
    }

    /// Copies all of `source` into `dest` with its top-left at `at`.
    pub(crate) fn copy_into(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: Slot,
        dest: Slot,
        at: (u32, u32),
    ) {
        let (width, height) = self.pool[source.0].extent;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("xui-vello copy"),
        });
        encoder.copy_texture_to_texture(
            self.pool[source.0].texture.as_image_copy(),
            wgpu::TexelCopyTextureInfo {
                texture: &self.pool[dest.0].texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: at.0, y: at.1, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
    }

    pub(crate) fn mask_count(&self) -> u64 {
        self.masks.entry_count()
    }

    /// An encoded texture of exactly `extent`: a Vello target, a render
    /// attachment, a sampled input and a copy source.
    pub(crate) fn acquire(&mut self, device: &wgpu::Device, extent: (u32, u32)) -> Slot {
        self.acquire_in(device, extent, Space::Encoded)
    }

    /// A texture of exactly `extent` in `space`. Only an encoded one can be a
    /// Vello target; both can be rendered to, sampled, and copied from.
    pub(crate) fn acquire_in(
        &mut self,
        device: &wgpu::Device,
        extent: (u32, u32),
        space: Space,
    ) -> Slot {
        if let Some(index) = self.pool.entries.iter().position(|entry| {
            entry.as_ref().is_some_and(|entry| {
                !entry.busy && !entry.pinned && entry.extent == extent && entry.space == space
            })
        }) {
            self.pool[index].busy = true;
            self.pool[index].last_used = self.frame;
            return Slot(index);
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("xui-vello filter texture"),
            size: wgpu::Extent3d {
                width: extent.0,
                height: extent.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: space.format(),
            usage: match space {
                // Vello writes these with a compute shader.
                Space::Encoded => wgpu::TextureUsages::STORAGE_BINDING,
                Space::Work => wgpu::TextureUsages::empty(),
            } | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Slot(self.pool.insert(Pooled {
            texture,
            view,
            extent,
            space,
            busy: true,
            pinned: false,
            last_used: self.frame,
        }))
    }

    pub(crate) fn release(&mut self, slot: Slot) {
        self.pool[slot.0].busy = false;
    }

    pub(crate) fn view(&self, slot: Slot) -> &wgpu::TextureView {
        &self.pool[slot.0].view
    }

    pub(crate) fn texture(&self, slot: Slot) -> &wgpu::Texture {
        &self.pool[slot.0].texture
    }

    fn pass(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        params: Params,
        first: Slot,
        second: Option<Input>,
        extent: (u32, u32),
    ) -> Slot {
        self.pass3(device, encoder, params, first, second, None, extent)
    }

    #[allow(clippy::too_many_arguments)]
    fn pass3(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        params: Params,
        first: Slot,
        second: Option<Input>,
        third: Option<Input>,
        extent: (u32, u32),
    ) -> Slot {
        let out = self.acquire_in(device, extent, Space::Work);
        self.encode_pass(
            device,
            encoder,
            params,
            &self.pool[first.0].view,
            second,
            third,
            &self.pool[out.0].view,
            Space::Work,
        );
        out
    }

    fn view_of<'a>(
        &'a self,
        mask: &'a Option<Mask>,
        input: Option<Input<'a>>,
    ) -> &'a wgpu::TextureView {
        match (mask, input) {
            (Some(mask), _) => &mask.view,
            (None, Some(Input::Slot(slot))) => &self.pool[slot.0].view,
            (None, Some(Input::View(view))) => view,
            _ => &self.dummy,
        }
    }

    /// One full-screen pass from up to three inputs into `out`.
    #[allow(clippy::too_many_arguments)]
    fn encode_pass(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        params: Params,
        first: &wgpu::TextureView,
        second: Option<Input>,
        third: Option<Input>,
        out: &wgpu::TextureView,
        out_space: Space,
    ) {
        self.encode_to(
            device,
            encoder,
            params,
            [Some(Input::View(first)), second, third, None],
            Target {
                view: out,
                space: out_space,
                blend: Blend::Replace,
                scissor: None,
            },
        );
    }

    /// One full-screen pass from up to three inputs, and a clip mask, into
    /// `target`. An input left out reads as a transparent texel.
    fn encode_to(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        params: Params,
        [first, second, third, clip_mask]: [Option<Input>; 4],
        target: Target<'_>,
    ) {
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("xui-vello filter params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        // A mask is looked up by value: the cache hands out clones.
        let mask_of = |input: Option<Input>| match input {
            Some(Input::Mask(id)) => self.masks.get(&id),
            _ => None,
        };
        let first_mask = mask_of(first);
        let second_mask = mask_of(second);
        let third_mask = mask_of(third);
        let first_view = self.view_of(&first_mask, first);
        let second_view = self.view_of(&second_mask, second);
        let third_view = self.view_of(&third_mask, third);
        let clip_mask_view = self.view_of(&None, clip_mask);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xui-vello filter"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(first_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(second_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(third_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(clip_mask_view),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("xui-vello filter"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: match target.blend {
                        Blend::Replace => wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        Blend::Over | Blend::Under | Blend::Overwrite => wgpu::LoadOp::Load,
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(match (target.space, target.blend) {
            (Space::Encoded, _) => &self.pipeline_encoded,
            (Space::Work, Blend::Replace | Blend::Overwrite) => &self.pipeline_work,
            (Space::Work, Blend::Over) => &self.pipeline_over,
            (Space::Work, Blend::Under) => &self.pipeline_under,
        });
        if let Some([x, y, width, height]) = target.scissor {
            pass.set_scissor_rect(x, y, width, height);
        }
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// A work texture of `extent` holding `color` (sRGB-encoded, straight)
    /// everywhere, premultiplied: where a composition starts when no band
    /// comes first.
    pub(crate) fn clear(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        extent: (u32, u32),
        color: [f32; 4],
    ) -> Slot {
        let out = self.acquire_in(device, extent, Space::Work);
        let alpha = color[3].clamp(0.0, 1.0) as f64;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("xui-vello clear"),
        });
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("xui-vello clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.pool[out.0].view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: color[0] as f64 * alpha,
                        g: color[1] as f64 * alpha,
                        b: color[2] as f64 * alpha,
                        a: alpha,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        queue.submit([encoder.finish()]);
        out
    }

    /// A band of 2D content, as Vello wrote it (straight alpha), becomes the
    /// start of the composition, premultiplied. `band` is released.
    pub(crate) fn premultiplied(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        band: Slot,
        extent: (u32, u32),
    ) -> Slot {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("xui-vello premultiply"),
        });
        let out = self.pass(device, &mut encoder, Params::op(0), band, None, extent);
        self.release(band);
        queue.submit([encoder.finish()]);
        out
    }

    /// `front` over `back`, both premultiplied and of `extent`, into a new
    /// texture. Neither is released.
    pub(crate) fn over(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        front: Slot,
        back: Slot,
        extent: (u32, u32),
    ) -> Slot {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("xui-vello over"),
        });
        let out = self.pass(device, &mut encoder, Params::op(24), front, Some(Input::Slot(back)), extent);
        queue.submit([encoder.finish()]);
        out
    }

    /// A copy of `slot`, in a texture of its own.
    pub(crate) fn duplicate(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, slot: Slot) -> Slot {
        let (extent, space) = (self.pool[slot.0].extent, self.pool[slot.0].space);
        let copy = self.acquire_in(device, extent, space);
        self.copy_into(device, queue, slot, copy, (0, 0));
        copy
    }

    /// A band of 2D content over what is composed so far: the source-over
    /// Vello itself would have done had the band not been cut. Both are
    /// released.
    pub(crate) fn band_over(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        band: Slot,
        below: Slot,
        extent: (u32, u32),
    ) -> Slot {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("xui-vello band"),
        });
        let out = self.pass(
            device,
            &mut encoder,
            Params::op(17),
            band,
            Some(Input::Slot(below)),
            extent,
        );
        self.release(band);
        self.release(below);
        queue.submit([encoder.finish()]);
        out
    }

    /// An external texture over what is composed so far, clipped by
    /// `draw.clips` and `clip_mask`. `below` is released.
    pub(crate) fn texture_over(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        below: Slot,
        draw: &TextureDraw,
        clip_mask: Option<Placed>,
        extent: (u32, u32),
    ) -> Slot {
        let mut params = Params::op(18);
        // Bit 0: what is sampled is linear light and must be encoded. An sRGB
        // format is decoded by the hardware as it is sampled, and a float one
        // is taken to hold linear light, as a renderer that wants more than
        // 8 bits writes it; anything else holds sRGB-encoded values already.
        let flags = (samples_linear(draw.texture.format()) as u32) | ((draw.nearest as u32) << 1);
        params.op[1] = flags;
        params.tex0 = [draw.inverse[0], draw.inverse[1], draw.inverse[2], draw.inverse[3]];
        params.tex1 = [draw.inverse[4], draw.inverse[5], draw.opacity, draw.clips.len() as f32];
        params.tex2 = draw.bounds;
        graph::set_clips(&mut params, &draw.clips);
        params.tex1[2] = draw.opacity;
        let mask = self.clip_mask_input(&mut params, clip_mask);
        // Sampled as it is: its own format decides how it is read.
        let view = draw
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("xui-vello texture"),
        });
        let out = self.acquire_in(device, extent, Space::Work);
        self.encode_to(
            device,
            &mut encoder,
            params,
            [Some(Input::Slot(below)), Some(Input::View(&view)), None, mask],
            Target {
                view: &self.pool[out.0].view,
                space: Space::Work,
                blend: Blend::Replace,
                scissor: None,
            },
        );
        self.release(below);
        queue.submit([encoder.finish()]);
        out
    }

    /// Binds `clip_mask` for the shader's `clip_coverage`: `mask_rect` says
    /// where it sits in the output's texels.
    fn clip_mask_input(
        &self,
        params: &mut Params,
        clip_mask: Option<Placed>,
    ) -> Option<Input<'static>> {
        let mask = clip_mask?;
        let (width, height) = self.pool[mask.slot.0].extent;
        params.mask_rect = [
            mask.origin.0,
            mask.origin.1,
            i32::try_from(width).unwrap_or(i32::MAX),
            i32::try_from(height).unwrap_or(i32::MAX),
        ];
        Some(Input::Slot(mask.slot))
    }

    /// The finished composition (premultiplied) written as Vello writes:
    /// straight alpha, 8 bits. `composed` is released.
    pub(crate) fn finish(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        composed: Slot,
        dest: Finish<'_>,
    ) {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("xui-vello finish"),
        });
        let out = match dest {
            Finish::Slot(slot) => &self.pool[slot.0].view,
            Finish::View(view) => view,
        };
        self.encode_pass(
            device,
            &mut encoder,
            Params::op(20),
            &self.pool[composed.0].view,
            None,
            None,
            out,
            Space::Encoded,
        );
        self.release(composed);
        queue.submit([encoder.finish()]);
    }

    fn ensure_mask(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &ImageData) {
        if data.size.width == 0
            || data.size.height == 0
            || data.pixels.len() != data.size.width as usize * data.size.height as usize * 4
        {
            return;
        }
        self.masks.get_with(data.id(), || {
            let size = wgpu::Extent3d {
                width: data.size.width,
                height: data.size.height,
                depth_or_array_layers: 1,
            };
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("xui-vello mask"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
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
                size,
            );
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            Mask {
                _texture: texture,
                view,
                bytes: u32::try_from(data.pixels.len()).unwrap_or(u32::MAX),
            }
        });
    }
}

#[derive(Clone, Copy)]
enum Input<'a> {
    Slot(Slot),
    Mask(ImageDataId),
    /// A texture that is not the pool's: an application's.
    View(&'a wgpu::TextureView),
}

/// `ColorMatrix` is 4x5 row-major; the shader wants its four rows then the
/// offset column.
fn matrix_rows(m: &[f32; 20]) -> [[f32; 4]; 5] {
    [
        [m[0], m[1], m[2], m[3]],
        [m[5], m[6], m[7], m[8]],
        [m[10], m[11], m[12], m[13]],
        [m[15], m[16], m[17], m[18]],
        [m[4], m[9], m[14], m[19]],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_shader_parses_as_wgsl() {
        naga::front::wgsl::parse_str(include_str!("filter.wgsl")).expect("filter.wgsl");
    }

    #[test]
    fn matrix_offsets_move_to_the_last_row() {
        let mut m = [0.0; 20];
        m[4] = 0.1;
        m[9] = 0.2;
        m[14] = 0.3;
        m[19] = 0.4;
        assert_eq!(super::matrix_rows(&m)[4], [0.1, 0.2, 0.3, 0.4]);
    }
}
