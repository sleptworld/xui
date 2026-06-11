use std::collections::HashMap;

use cosmic_text::{
    Attrs, Buffer, CacheKey, Family, FontSystem, LayoutGlyph, Metrics, Shaping,
    Style as CosmicStyle, SwashCache, Weight,
};
use xui_interface::{
    ComputedTextStyle, FontFamily, FontStyle, FontWeight, GlyphBitmap, GlyphPlacement, LineHeight,
    NodeId, NodeLifecycleEvent, Point, PositionedGlyph, Size, TextLayoutBackend,
    TextLayoutConstraints, TextMeasurer,
};

use super::rgba_bitmap_data;

pub struct CosmicTextEngine {
    font_system: FontSystem,
    swash_cache: SwashCache,
    node_cache: HashMap<NodeId, CachedNodeText>,
}

impl CosmicTextEngine {
    pub fn new() -> Self {
        Self {
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
            node_cache: HashMap::new(),
        }
    }

    pub fn font_system(&self) -> &FontSystem {
        &self.font_system
    }

    pub fn font_system_mut(&mut self) -> &mut FontSystem {
        &mut self.font_system
    }

    pub fn swash_cache(&self) -> &SwashCache {
        &self.swash_cache
    }

    pub fn swash_cache_mut(&mut self) -> &mut SwashCache {
        &mut self.swash_cache
    }

    pub fn clear_node_cache(&mut self) {
        self.node_cache.clear();
    }

    pub fn remove_node_cache(&mut self, id: NodeId) {
        self.node_cache.remove(&id);
    }

    pub fn cached_node_count(&self) -> usize {
        self.node_cache.len()
    }

    fn create_buffer(
        font_system: &mut FontSystem,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Buffer {
        let metrics = Metrics::new(
            style.font_size,
            line_height(style.line_height, style.font_size),
        );
        let attrs = attrs_for_style(style);
        let width = constraints
            .max_width
            .filter(|width| width.is_finite())
            .map(|width| width.max(0.0));

        let mut buffer = Buffer::new(font_system, metrics);
        buffer.set_size(width, None);
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(font_system, false);
        buffer
    }

    fn layout_from_buffer(buffer: &Buffer) -> CosmicTextLayout {
        let mut runs = Vec::new();
        let mut measured_width: f32 = 0.0;
        let mut measured_height: f32 = 0.0;

        for run in buffer.layout_runs() {
            measured_width = measured_width.max(run.line_w);
            measured_height = measured_height.max(run.line_top + run.line_height);
            runs.push(CosmicLayoutRun {
                line_y: run.line_y,
                line_top: run.line_top,
                line_height: run.line_height,
                line_w: run.line_w,
                glyphs: run.glyphs.to_vec(),
            });
        }

        CosmicTextLayout {
            runs,
            size: Size::<f32>::new(measured_width, measured_height),
        }
    }

    fn layout_text_uncached(
        &mut self,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> CosmicTextLayout {
        let buffer = Self::create_buffer(&mut self.font_system, text, style, constraints);
        Self::layout_from_buffer(&buffer)
    }
}

impl Default for CosmicTextEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TextMeasurer for CosmicTextEngine {
    fn measure_text(&mut self, text: &str, props: &ComputedTextStyle) -> Size<f32> {
        self.measure_text_with_constraints(text, props, TextLayoutConstraints::UNBOUNDED)
    }

    fn measure_text_with_constraints(
        &mut self,
        text: &str,
        props: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Size<f32> {
        self.layout_text_uncached(text, props, constraints).size
    }

    fn measure_node_text(
        &mut self,
        node_id: NodeId,
        text: &str,
        props: &ComputedTextStyle,
    ) -> Size<f32> {
        self.measure_node_text_with_constraints(
            node_id,
            text,
            props,
            TextLayoutConstraints::UNBOUNDED,
        )
    }

    fn measure_node_text_with_constraints(
        &mut self,
        node_id: NodeId,
        text: &str,
        props: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Size<f32> {
        let layout = self.layout_node_text(node_id, text, props, constraints);
        layout.size
    }

    fn handle_node_lifecycle(&mut self, event: &NodeLifecycleEvent) {
        if let NodeLifecycleEvent::Removed(id) = event {
            self.node_cache.remove(id);
        }
    }
}

impl TextLayoutBackend for CosmicTextEngine {
    type Layout = CosmicTextLayout;
    type GlyphKey = CacheKey;

    fn layout_text(
        &mut self,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Self::Layout {
        self.layout_text_uncached(text, style, constraints)
    }

    fn layout_node_text(
        &mut self,
        node_id: NodeId,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Self::Layout {
        let key = CosmicTextLayoutKey::new(text, style, constraints);
        if let Some(cached) = self.node_cache.get(&node_id) {
            if cached.key == key {
                return cached.layout.clone();
            }
        }

        let layout = if let Some(cached) = self.node_cache.get_mut(&node_id) {
            update_buffer(
                &mut self.font_system,
                &mut cached.buffer,
                text,
                style,
                constraints,
            );
            cached.key = key;
            cached.layout = Self::layout_from_buffer(&cached.buffer);
            cached.layout.clone()
        } else {
            let buffer = Self::create_buffer(&mut self.font_system, text, style, constraints);
            let layout = Self::layout_from_buffer(&buffer);
            self.node_cache.insert(
                node_id,
                CachedNodeText {
                    key,
                    buffer,
                    layout: layout.clone(),
                },
            );
            layout
        };
        layout
    }

    fn get_cached_layout(&self, node_id: NodeId) -> Option<Self::Layout> {
        self.node_cache
            .get(&node_id)
            .map(|cached| cached.layout.clone())
    }

    fn layout_size(&self, layout: &Self::Layout) -> Size<f32> {
        layout.size
    }

    fn visit_layout_glyphs(
        &self,
        layout: &Self::Layout,
        origin: Point,
        scale_factor: f32,
        visitor: &mut dyn FnMut(PositionedGlyph<Self::GlyphKey>),
    ) {
        let physical_origin_x = origin.x * scale_factor;
        let physical_origin_y = origin.y * scale_factor;

        for run in &layout.runs {
            for glyph in &run.glyphs {
                let physical = glyph.physical(
                    (
                        physical_origin_x,
                        physical_origin_y + run.line_y * scale_factor,
                    ),
                    scale_factor,
                );
                visitor(PositionedGlyph {
                    key: physical.cache_key,
                    physical_x: physical.x,
                    physical_y: physical.y,
                });
            }
        }
    }

    fn rasterize_glyph(&mut self, key: &Self::GlyphKey) -> Option<GlyphBitmap> {
        let image = self
            .swash_cache
            .get_image(&mut self.font_system, *key)
            .clone()?;
        let placement = image.placement;

        let (data, ptype) = rgba_bitmap_data(image.content, &image.data);
        Some(GlyphBitmap {
            ptype,
            data,
            width: placement.width as u32,
            height: placement.height as u32,
            placement: GlyphPlacement {
                left: placement.left,
                top: placement.top,
                width: placement.width as u32,
                height: placement.height as u32,
            },
        })
    }
}

#[derive(Clone, Debug)]
pub struct CosmicTextLayout {
    runs: Vec<CosmicLayoutRun>,
    size: Size<f32>,
}

#[derive(Clone, Debug)]
struct CosmicLayoutRun {
    #[allow(dead_code)]
    line_top: f32,
    line_y: f32,
    #[allow(dead_code)]
    line_height: f32,
    #[allow(dead_code)]
    line_w: f32,
    glyphs: Vec<LayoutGlyph>,
}

struct CachedNodeText {
    key: CosmicTextLayoutKey,
    buffer: Buffer,
    layout: CosmicTextLayout,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CosmicTextLayoutKey {
    text: Box<str>,
    style: CosmicTextStyleKey,
    constraints: CosmicTextLayoutConstraintsKey,
}

impl CosmicTextLayoutKey {
    fn new(text: &str, style: &ComputedTextStyle, constraints: TextLayoutConstraints) -> Self {
        Self {
            text: Box::from(text),
            style: CosmicTextStyleKey::from(style),
            constraints: CosmicTextLayoutConstraintsKey::from(constraints),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct CosmicTextLayoutConstraintsKey {
    max_width: Option<F32Key>,
}

impl From<TextLayoutConstraints> for CosmicTextLayoutConstraintsKey {
    fn from(constraints: TextLayoutConstraints) -> Self {
        Self {
            max_width: constraints.max_width.map(|width| F32Key(width.to_bits())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CosmicTextStyleKey {
    font_family: FontFamily,
    font_size: F32Key,
    font_weight: FontWeight,
    font_style: FontStyle,
    line_height: LineHeightKey,
    letter_spacing: F32Key,
    decoration: xui_interface::TextDecoration,
}

impl From<&ComputedTextStyle> for CosmicTextStyleKey {
    fn from(style: &ComputedTextStyle) -> Self {
        Self {
            font_family: style.font_family.clone(),
            font_size: F32Key(style.font_size.to_bits()),
            font_weight: style.font_weight,
            font_style: style.font_style,
            line_height: LineHeightKey::from(style.line_height),
            letter_spacing: F32Key(style.letter_spacing.to_bits()),
            decoration: style.decoration,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct F32Key(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum LineHeightKey {
    Normal,
    Px(F32Key),
    Em(F32Key),
}

impl From<LineHeight> for LineHeightKey {
    fn from(line_height: LineHeight) -> Self {
        match line_height {
            LineHeight::Normal => Self::Normal,
            LineHeight::Px(value) => Self::Px(F32Key(value.to_bits())),
            LineHeight::Em(value) => Self::Em(F32Key(value.to_bits())),
        }
    }
}

fn update_buffer(
    font_system: &mut FontSystem,
    buffer: &mut Buffer,
    text: &str,
    style: &ComputedTextStyle,
    constraints: TextLayoutConstraints,
) {
    let metrics = Metrics::new(
        style.font_size,
        line_height(style.line_height, style.font_size),
    );
    let attrs = attrs_for_style(style);
    let width = constraints
        .max_width
        .filter(|width| width.is_finite())
        .map(|width| width.max(0.0));

    buffer.set_metrics_and_size(metrics, width, None);
    buffer.set_text(text, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);
}

fn attrs_for_style(style: &ComputedTextStyle) -> Attrs<'_> {
    let mut attrs = Attrs::new()
        .family(family(&style.font_family))
        .style(font_style(style.font_style))
        .weight(font_weight(style.font_weight));

    if style.decoration.underline {
        attrs = attrs.underline(cosmic_text::UnderlineStyle::Single);
    }

    attrs
}

fn family(family: &FontFamily) -> Family<'_> {
    match family {
        FontFamily::System => Family::SansSerif,
        FontFamily::Named(name) => Family::Name(name),
        FontFamily::Stack(names) => names
            .first()
            .map(|name| Family::Name(name.as_str()))
            .unwrap_or(Family::SansSerif),
    }
}

fn font_weight(weight: FontWeight) -> Weight {
    match weight {
        FontWeight::Thin => Weight(100),
        FontWeight::ExtraLight => Weight(200),
        FontWeight::Light => Weight(300),
        FontWeight::Normal => Weight::NORMAL,
        FontWeight::Medium => Weight(500),
        FontWeight::SemiBold => Weight(600),
        FontWeight::Bold => Weight::BOLD,
        FontWeight::ExtraBold => Weight(800),
        FontWeight::Black => Weight(900),
        FontWeight::Number(value) => Weight(value.clamp(1, 1000)),
    }
}

fn font_style(style: FontStyle) -> CosmicStyle {
    match style {
        FontStyle::Normal => CosmicStyle::Normal,
        FontStyle::Italic => CosmicStyle::Italic,
        FontStyle::Oblique => CosmicStyle::Oblique,
    }
}

fn line_height(line_height: LineHeight, font_size: f32) -> f32 {
    match line_height {
        LineHeight::Normal => font_size,
        LineHeight::Px(px) => px,
        LineHeight::Em(em) => em * font_size,
    }
    .max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xui_interface::TextStyle;

    fn text_style() -> ComputedTextStyle {
        TextStyle::default().into()
    }

    #[test]
    fn node_measurement_populates_node_cache() {
        let mut engine = CosmicTextEngine::new();
        let node_id = NodeId::default();
        let style = text_style();

        assert_eq!(engine.cached_node_count(), 0);
        assert!(engine.get_cached_layout(node_id).is_none());

        let first = engine.measure_node_text_with_constraints(
            node_id,
            "cached",
            &style,
            TextLayoutConstraints::UNBOUNDED,
        );
        let second = engine.measure_node_text_with_constraints(
            node_id,
            "cached",
            &style,
            TextLayoutConstraints::UNBOUNDED,
        );

        assert_eq!(first, second);
        assert_eq!(engine.cached_node_count(), 1);
        assert!(engine.get_cached_layout(node_id).is_some());
    }

    #[test]
    fn removed_node_releases_node_cache() {
        let mut engine = CosmicTextEngine::new();
        let node_id = NodeId::default();
        let style = text_style();

        engine.measure_node_text(node_id, "cached", &style);
        engine.handle_node_lifecycle(&NodeLifecycleEvent::Removed(node_id));

        assert_eq!(engine.cached_node_count(), 0);
        assert!(engine.get_cached_layout(node_id).is_none());
    }
}
