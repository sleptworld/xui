//! Draws xui glyph runs with `wgpu`.
//!
//! A text backend's [`GlyphRasterizer`] turns glyphs into bitmaps; this crate
//! decides where those bitmaps live on the GPU and draws them:
//!
//! - [`GlyphAtlas`] manages the space of fixed-size texture pages with
//!   `etagere`: it gives each new glyph a rectangle and evicts the glyphs a
//!   frame no longer uses.
//! - [`TextRenderer`] owns the page textures, writes each new glyph's bitmap
//!   straight into its rectangle, and draws glyph quads with its own shader,
//!   in the order and place the caller asks for. No copy of the pages is kept
//!   on the CPU. The solid rectangles around text (underlines, carets) are
//!   drawn with it, so they keep their place in the paint order.
//!
//! A frame goes: [`begin_frame`](TextRenderer::begin_frame), one
//! [`prepare`](TextRenderer::prepare) per group of draws that is drawn at one
//! point of the paint order, a single [`upload`](TextRenderer::upload) of the
//! frame's quads, then [`render`](TextRenderer::render) each prepared
//! [`TextBatch`] into whichever render pass it belongs to:
//!
//! ```ignore
//! renderer.begin_frame(fonts.epoch());
//! let batch = renderer.prepare(&device, &queue, &*rasterizer, target, &[
//!     TextDraw::Glyphs(GlyphRunDraw {
//!         font: run.font_id,
//!         ppem: run.font_size * scale,
//!         color,
//!         clip: None,
//!         glyphs: &positioned,
//!     }),
//! ]);
//! renderer.upload(&device, &queue);
//! renderer.render(&mut pass, &batch, format);
//! ```
//!
//! Glyphs are drawn as pixel-aligned bitmaps, so this suits text whose
//! transform is a translation (any scale is folded into `ppem`). Rotated or
//! skewed text, and text cut by non-rectangular clips, is for a vector path.
//!
//! # Colour
//!
//! Colours are given sRGB-encoded with straight alpha, and text blends on
//! encoded values, as the 2D renderers it is drawn among do: into any target
//! the values are written as they are, whatever its format holds them in
//! (`Rgba8Unorm`, or a float format a compositor keeps its encoded values in).
//! The one exception is an sRGB format, which the hardware encodes as it is
//! written: there colours are decoded first, so an opaque colour lands as it
//! was given. Blending into such a target is the hardware's, in linear light;
//! nothing in this crate can change that, so a renderer that wants text to
//! match its own blending should not draw into one.

mod atlas;

use std::{collections::HashMap, hash::Hash, ops::Range};

use bytemuck::{Pod, Zeroable};
use xui_interface::{GlyphId, GlyphImage, GlyphRasterizer, RasterizedGlyphFormat};

pub use atlas::{
    AtlasFormat, AtlasFull, AtlasGlyph, AtlasOptions, AtlasPage, GlyphAtlas, GlyphLookup,
    PlacedGlyph,
};

/// The texture a batch is drawn into, as positions see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextTarget {
    /// Where the target's texel (0, 0) is, in the physical pixels positions
    /// and clips are given in.
    pub origin_px: (i32, i32),
    /// Width and height in texels.
    pub extent: (u32, u32),
}

/// A glyph at its pen position, in physical pixels, `y` on the baseline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionedGlyph {
    pub id: GlyphId,
    pub x: f32,
    pub y: f32,
}

/// Glyphs of one font at one size and colour.
#[derive(Debug, Clone, Copy)]
pub struct GlyphRunDraw<'a, F> {
    pub font: F,
    /// Physical pixels per em: the logical font size times every scale on the
    /// way to the target.
    pub ppem: f32,
    /// sRGB-encoded, straight alpha.
    pub color: [f32; 4],
    /// `[x0, y0, x1, y1]` in the same pixels as the glyph positions; nothing
    /// outside is drawn. `None` clips to the target only.
    pub clip: Option<[f32; 4]>,
    pub glyphs: &'a [PositionedGlyph],
}

/// A solid rectangle, antialiased where its edges fall inside a pixel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectDraw {
    /// `[x0, y0, x1, y1]` in physical pixels.
    pub rect: [f32; 4],
    /// sRGB-encoded, straight alpha.
    pub color: [f32; 4],
    pub clip: Option<[f32; 4]>,
}

/// One thing drawn by [`TextRenderer::prepare`], in order.
#[derive(Debug, Clone, Copy)]
pub enum TextDraw<'a, F> {
    Glyphs(GlyphRunDraw<'a, F>),
    Rect(RectDraw),
}

/// What [`TextRenderer::prepare`] laid out, drawn at one point of the paint
/// order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextBatch {
    draws: Vec<Draw>,
    incomplete: bool,
}

impl TextBatch {
    pub fn is_empty(&self) -> bool {
        self.draws.is_empty()
    }

    /// Whether a glyph with ink was left out because the atlas had no room
    /// for it ([`AtlasFull`]). Such a batch draws text with holes in it: a
    /// caller with another way to draw the text should use that instead.
    pub fn incomplete(&self) -> bool {
        self.incomplete
    }

    /// Draws `other` after what is already in `self`, as one batch.
    pub fn append(&mut self, other: TextBatch) {
        self.incomplete |= other.incomplete;
        for draw in other.draws {
            self.push_finished(draw);
        }
    }

    /// Adds `draw` to the batch [`prepare`](TextRenderer::prepare) is
    /// building, joining the last draw whenever the pages they bind agree.
    fn push(&mut self, draw: Draw) {
        if let Some(last) = self.draws.last_mut()
            && let Some(pages) = last.joined(&draw)
        {
            (last.alpha, last.color) = pages;
            last.instances.end = draw.instances.end;
            return;
        }
        self.draws.push(draw);
    }

    /// Adds a draw of another, finished batch: joined only to bind a pair of
    /// pages one of the two binds already, which has its bind group made.
    fn push_finished(&mut self, draw: Draw) {
        if let Some(last) = self.draws.last_mut()
            && let Some(pages) = last.joined(&draw)
            && (pages == (last.alpha, last.color) || pages == (draw.alpha, draw.color))
        {
            (last.alpha, last.color) = pages;
            last.instances.end = draw.instances.end;
            return;
        }
        self.draws.push(draw);
    }
}

/// Consecutive instances drawn with one pair of pages bound, a coverage page
/// and a colour page: one draw call. Rectangles sample neither, so they join
/// any draw.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Draw {
    alpha: Option<usize>,
    color: Option<usize>,
    instances: Range<u32>,
}

impl Draw {
    /// The pages `self` and the `next` draw bind together, if they can be one
    /// draw call: they follow each other and bind no different pages.
    fn joined(&self, next: &Draw) -> Option<(Option<usize>, Option<usize>)> {
        let agree = |a: Option<usize>, b: Option<usize>| a.is_none() || b.is_none() || a == b;
        (self.instances.end == next.instances.start
            && agree(self.alpha, next.alpha)
            && agree(self.color, next.color))
        .then(|| (self.alpha.or(next.alpha), self.color.or(next.color)))
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Instance {
    rect: [f32; 4],
    texels: [f32; 4],
    color: [f32; 4],
    clip: [f32; 4],
    kind: u32,
}

const MASK: u32 = 0;
const SUBPIXEL_MASK: u32 = 1;
const COLOR: u32 = 2;
const RECT: u32 = 3;

/// One atlas page on the GPU.
struct GpuPage {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// Draws glyph runs and rectangles into `wgpu` render passes, from an atlas
/// it keeps on the GPU. `F` is the font id type of the rasterizer glyphs come
/// from.
///
/// Draws blend premultiplied colour over the target. Each draw call binds a
/// coverage page and a colour page together, so text mixing masks and colour
/// glyphs (emoji in a sentence) is split into draw calls only where it moves
/// to another page of one format.
pub struct TextRenderer<F> {
    atlas: GlyphAtlas<F>,
    /// One per target format the renderer was made for.
    pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    layout: wgpu::BindGroupLayout,
    /// Bound where a draw samples no page of a format.
    no_page: wgpu::TextureView,
    /// By atlas page index: the texture of the page with that id.
    pages: Vec<Option<(u64, GpuPage)>>,
    /// By the ids of the coverage and colour pages a draw binds.
    bind_groups: HashMap<(Option<u64>, Option<u64>), wgpu::BindGroup>,
    instances: Vec<Instance>,
    instance_buffer: Option<wgpu::Buffer>,
    /// Instances in `instance_buffer`, which `render` can draw.
    uploaded: usize,
}

/// Whether the hardware encodes what is written to a target of `format`, so
/// the shader has to write linear light for the stored values to come out
/// encoded.
fn is_linear(format: wgpu::TextureFormat) -> bool {
    format.is_srgb()
}

impl<F: Copy + Eq + Hash> TextRenderer<F> {
    /// A renderer for render passes whose colour attachment is one of
    /// `formats`. Pages are no larger than `device` allows.
    pub fn new(
        device: &wgpu::Device,
        formats: &[wgpu::TextureFormat],
        options: AtlasOptions,
    ) -> Self {
        let options = AtlasOptions {
            page_size: options
                .page_size
                .min(device.limits().max_texture_dimension_2d),
            ..options
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("xui-wgpu-text page"),
            entries: &[0, 1].map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }),
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xui-wgpu-text"),
            source: wgpu::ShaderSource::Wgsl(include_str!("text.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xui-wgpu-text"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipelines = formats
            .iter()
            .map(|&format| {
                let constants = [("LINEAR", if is_linear(format) { 1.0 } else { 0.0 })];
                let compilation_options = wgpu::PipelineCompilationOptions {
                    constants: &constants,
                    ..Default::default()
                };
                let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("xui-wgpu-text"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_main"),
                        compilation_options: compilation_options.clone(),
                        buffers: &[Some(wgpu::VertexBufferLayout {
                            array_stride: size_of::<Instance>() as u64,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &wgpu::vertex_attr_array![
                                0 => Float32x4,
                                1 => Float32x4,
                                2 => Float32x4,
                                3 => Float32x4,
                                4 => Uint32,
                            ],
                        })],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_main"),
                        compilation_options,
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                });
                (format, pipeline)
            })
            .collect();
        let no_page = page_texture(
            device,
            wgpu::TextureFormat::R8Unorm,
            1,
            "xui-wgpu-text no page",
        )
        .view;
        Self {
            atlas: GlyphAtlas::new(options),
            pipelines,
            layout,
            no_page,
            pages: Vec::new(),
            bind_groups: HashMap::new(),
            instances: Vec::new(),
            instance_buffer: None,
            uploaded: 0,
        }
    }

    pub fn atlas(&self) -> &GlyphAtlas<F> {
        &self.atlas
    }

    /// Starts a frame: batches of the previous one can no longer be drawn.
    /// `font_epoch` is the font collection's current epoch. The textures of
    /// pages the atlas closes are released.
    pub fn begin_frame(&mut self, font_epoch: u64) {
        self.atlas.begin_frame(font_epoch);
        for (index, page) in self.pages.iter_mut().enumerate() {
            if page
                .as_ref()
                .is_some_and(|(id, _)| self.atlas.page(index).map(AtlasPage::id) != Some(*id))
            {
                *page = None;
            }
        }
        let pages = &self.pages;
        let open = |id: &Option<u64>| {
            id.is_none_or(|id| pages.iter().flatten().any(|(open, _)| *open == id))
        };
        self.bind_groups
            .retain(|(alpha, color), _| open(alpha) && open(color));
        self.instances.clear();
        self.uploaded = 0;
    }

    /// Lays `draws` out for `target`, in order. Glyphs the atlas lacks are
    /// rasterized and written into their page textures through `queue` now.
    /// Glyphs without ink, anything outside its clip, and glyphs the atlas
    /// has no room for this frame are left out.
    pub fn prepare<R: GlyphRasterizer<FontId = F> + ?Sized>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rasterizer: &R,
        target: TextTarget,
        draws: &[TextDraw<'_, F>],
    ) -> TextBatch {
        let origin = (target.origin_px.0 as f32, target.origin_px.1 as f32);
        let (width, height) = (target.extent.0 as f32, target.extent.1 as f32);
        // Target pixels to clip space.
        let ndc = |[x0, y0, x1, y1]: [f32; 4]| {
            [
                x0 / width * 2.0 - 1.0,
                1.0 - y0 / height * 2.0,
                x1 / width * 2.0 - 1.0,
                1.0 - y1 / height * 2.0,
            ]
        };
        // Into target pixels, cut to the target.
        let target_clip = |clip: Option<[f32; 4]>| {
            let [x0, y0, x1, y1] = clip.map_or([0.0, 0.0, width, height], |[x0, y0, x1, y1]| {
                [
                    (x0 - origin.0).max(0.0),
                    (y0 - origin.1).max(0.0),
                    (x1 - origin.0).min(width),
                    (y1 - origin.1).min(height),
                ]
            });
            (x0 < x1 && y0 < y1).then_some([x0, y0, x1, y1])
        };
        let outside = |[x0, y0, x1, y1]: [f32; 4], clip: [f32; 4]| {
            x1 <= clip[0] || y1 <= clip[1] || x0 >= clip[2] || y0 >= clip[3]
        };

        let mut batch = TextBatch::default();
        for draw in draws {
            match draw {
                TextDraw::Glyphs(run) => {
                    let Some(clip) = target_clip(run.clip) else {
                        continue;
                    };
                    for glyph in run.glyphs {
                        let lookup = match self.atlas.glyph(
                            rasterizer,
                            run.font,
                            glyph.id,
                            run.ppem,
                            glyph.x - origin.0,
                            glyph.y - origin.1,
                        ) {
                            Ok(Some(lookup)) => lookup,
                            Ok(None) => continue,
                            Err(AtlasFull) => {
                                batch.incomplete = true;
                                continue;
                            }
                        };
                        let placed = lookup.placed;
                        let atlas = placed.glyph;
                        if let Some(image) = &lookup.upload {
                            self.write_glyph(device, queue, &atlas, image);
                        }
                        let (x0, y0) = (placed.x as f32, placed.y as f32);
                        let quad = [x0, y0, x0 + atlas.width as f32, y0 + atlas.height as f32];
                        if outside(quad, clip) {
                            continue;
                        }
                        let index = self.instances.len() as u32;
                        self.instances.push(Instance {
                            rect: ndc(quad),
                            texels: [
                                atlas.x as f32,
                                atlas.y as f32,
                                (atlas.x + atlas.width) as f32,
                                (atlas.y + atlas.height) as f32,
                            ],
                            color: run.color,
                            clip,
                            kind: match atlas.format {
                                RasterizedGlyphFormat::Mask => MASK,
                                RasterizedGlyphFormat::SubpixelMask => SUBPIXEL_MASK,
                                RasterizedGlyphFormat::Color => COLOR,
                            },
                        });
                        let page = Some(atlas.page);
                        let (alpha, color) = match atlas.format {
                            RasterizedGlyphFormat::Mask => (page, None),
                            RasterizedGlyphFormat::SubpixelMask | RasterizedGlyphFormat::Color => {
                                (None, page)
                            }
                        };
                        batch.push(Draw {
                            alpha,
                            color,
                            instances: index..index + 1,
                        });
                    }
                }
                TextDraw::Rect(rect) => {
                    let Some(clip) = target_clip(rect.clip) else {
                        continue;
                    };
                    let [x0, y0, x1, y1] = rect.rect;
                    let edges = [x0 - origin.0, y0 - origin.1, x1 - origin.0, y1 - origin.1];
                    // Every pixel the rectangle touches.
                    let quad = [
                        edges[0].floor(),
                        edges[1].floor(),
                        edges[2].ceil(),
                        edges[3].ceil(),
                    ];
                    if !(edges[0] < edges[2] && edges[1] < edges[3]) || outside(quad, clip) {
                        continue;
                    }
                    let index = self.instances.len() as u32;
                    self.instances.push(Instance {
                        rect: ndc(quad),
                        texels: edges,
                        color: rect.color,
                        clip,
                        kind: RECT,
                    });
                    batch.push(Draw {
                        alpha: None,
                        color: None,
                        instances: index..index + 1,
                    });
                }
            }
        }
        for draw in &batch.draws {
            self.bind_group(device, draw);
        }
        batch
    }

    /// Makes the bind group of the pages `draw` binds, if it is not made.
    fn bind_group(&mut self, device: &wgpu::Device, draw: &Draw) {
        let key = self.bind_key(draw);
        if self.bind_groups.contains_key(&key) {
            return;
        }
        let view = |index: Option<usize>| match index {
            Some(index) => &self.pages[index].as_ref().expect("a written page").1.view,
            None => &self.no_page,
        };
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xui-wgpu-text pages"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view(draw.alpha)),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(view(draw.color)),
                },
            ],
        });
        self.bind_groups.insert(key, bind_group);
    }

    fn bind_key(&self, draw: &Draw) -> (Option<u64>, Option<u64>) {
        let id = |index: Option<usize>| {
            index.map(|index| self.pages[index].as_ref().expect("a written page").0)
        };
        (id(draw.alpha), id(draw.color))
    }

    /// Writes `image` into `glyph`'s rectangle of its page, creating the
    /// page's texture if the atlas opened it since the last write.
    fn write_glyph(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        glyph: &AtlasGlyph,
        image: &GlyphImage,
    ) {
        let page = self
            .atlas
            .page(glyph.page)
            .expect("an atlas glyph is on an open page");
        if self.pages.len() <= glyph.page {
            self.pages.resize_with(glyph.page + 1, || None);
        }
        if !matches!(&self.pages[glyph.page], Some((id, _)) if *id == page.id()) {
            let format = match page.format() {
                AtlasFormat::Alpha8 => wgpu::TextureFormat::R8Unorm,
                AtlasFormat::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
            };
            let texture = page_texture(device, format, page.size(), "xui-wgpu-text page");
            self.pages[glyph.page] = Some((page.id(), texture));
        }
        let (_, texture) = self.pages[glyph.page].as_ref().expect("made above");
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: glyph.x,
                    y: glyph.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &image.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(image.stride() as u32),
                rows_per_image: Some(image.height),
            },
            wgpu::Extent3d {
                width: image.width,
                height: image.height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Uploads the frame's quads. Call it once, after the frame's last
    /// `prepare` and before its first `render`.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.instances.is_empty() {
            self.uploaded = 0;
            return;
        }
        let bytes: &[u8] = bytemuck::cast_slice(&self.instances);
        let fits = self
            .instance_buffer
            .as_ref()
            .is_some_and(|buffer| buffer.size() >= bytes.len() as u64);
        if !fits {
            self.instance_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("xui-wgpu-text instances"),
                size: (bytes.len() as u64).next_power_of_two(),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        if let Some(buffer) = &self.instance_buffer {
            queue.write_buffer(buffer, 0, bytes);
        }
        self.uploaded = self.instances.len();
    }

    /// Draws `batch` into `pass`, whose colour attachment is `format` (one
    /// the renderer was made for) and has the extent `prepare` was given.
    ///
    /// # Panics
    ///
    /// If the renderer was not made for `format`.
    pub fn render(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        batch: &TextBatch,
        format: wgpu::TextureFormat,
    ) {
        let Some(buffer) = &self.instance_buffer else {
            return;
        };
        if batch.is_empty() {
            return;
        }
        let pipeline = self
            .pipelines
            .iter()
            .find_map(|(made_for, pipeline)| (*made_for == format).then_some(pipeline))
            .expect("TextRenderer::render into a format it was not made for");
        pass.set_pipeline(pipeline);
        pass.set_vertex_buffer(0, buffer.slice(..));
        for draw in &batch.draws {
            debug_assert!(
                draw.instances.end as usize <= self.uploaded,
                "TextRenderer::render before upload"
            );
            let bind_group = &self.bind_groups[&self.bind_key(draw)];
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..6, draw.instances.clone());
        }
    }
}

fn page_texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    size: u32,
    label: &str,
) -> GpuPage {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    GpuPage { texture, view }
}

#[cfg(test)]
mod tests;
