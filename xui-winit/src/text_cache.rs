use std::collections::HashMap;
use std::sync::Arc;

use xui_interface::{
    Color, ComputedTextStyle, FontFamily, FontStyle, FontWeight, LineHeight, NodeId,
    NodeLifecycleEvent, PositionedGlyph, Size, TextDecoration, TextLayoutBackend,
    TextLayoutConstraints, TextMeasurer,
};
use xui_text::engine::Engine;
use xui_interface::GlyphBitmap;
use crate::CosmicTextEngine;

pub struct WinitTextEngine<T: TextLayoutBackend = Engine> {
    inner: T,
    layout_cache: HashMap<NodeId, CachedNodeLayout<T::Layout>>,
}

impl WinitTextEngine<Engine> {
    pub fn new() -> Self {
        Self::with_layouter(Engine::new())
    }
}

impl WinitTextEngine<CosmicTextEngine> {
    pub fn new() -> Self {
        Self::with_layouter(CosmicTextEngine::new())
    }
}

impl<T: TextLayoutBackend> WinitTextEngine<T> {
    pub fn with_layouter(inner: T) -> Self {
        Self {
            inner,
            layout_cache: HashMap::new(),
        }
    }

    pub fn inner(&self) -> &T {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    pub fn into_inner(self) -> T {
        self.inner
    }

    pub fn clear_layout_cache(&mut self) {
        self.layout_cache.clear();
    }

    pub fn remove_node_layout_cache(&mut self, id: NodeId) {
        self.layout_cache.remove(&id);
    }

    pub fn cached_layout_count(&self) -> usize {
        self.layout_cache.len()
    }
}

impl Default for WinitTextEngine<Engine> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: TextLayoutBackend> TextMeasurer for WinitTextEngine<T> {
    fn measure_text(&mut self, text: &str, props: &ComputedTextStyle) -> Size<f32> {
        self.inner.measure_text(text, props)
    }

    fn measure_text_with_constraints(
        &mut self,
        text: &str,
        props: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Size<f32> {
        self.inner
            .measure_text_with_constraints(text, props, constraints)
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
        self.inner.layout_size(&layout)
    }

    fn handle_node_lifecycle(&mut self, event: &NodeLifecycleEvent) {
        if let NodeLifecycleEvent::Removed(id) = event {
            self.layout_cache.remove(id);
        }
        self.inner.handle_node_lifecycle(event);
    }
}

impl<T: TextLayoutBackend> TextLayoutBackend for WinitTextEngine<T> {
    type Layout = T::Layout;
    type GlyphKey = T::GlyphKey;

    fn layout_text(
        &mut self,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Self::Layout {
        self.inner.layout_text(text, style, constraints)
    }

    fn layout_node_text(
        &mut self,
        node_id: NodeId,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Self::Layout {
        let key = TextLayoutKey::new(text, style, constraints);
        if let Some(cached) = self.layout_cache.get(&node_id) {
            if cached.key == key {
                return cached.layout.clone();
            }
        }

        let layout = self
            .inner
            .layout_node_text(node_id, text, style, constraints);
        self.layout_cache.insert(
            node_id,
            CachedNodeLayout {
                key,
                layout: layout.clone(),
            },
        );
        layout
    }

    fn get_cached_layout(&self, node_id: NodeId) -> Option<Self::Layout> {
        self.layout_cache
            .get(&node_id)
            .map(|cached| cached.layout.clone())
    }

    fn layout_size(&self, layout: &Self::Layout) -> Size<f32> {
        self.inner.layout_size(layout)
    }

    fn visit_layout_glyphs(
        &self,
        layout: &Self::Layout,
        origin: xui_interface::Point,
        scale_factor: f32,
        visitor: &mut dyn FnMut(PositionedGlyph<Self::GlyphKey>),
    ) {
        self.inner
            .visit_layout_glyphs(layout, origin, scale_factor, visitor);
    }

    fn rasterize_glyph(&mut self, key: &Self::GlyphKey) -> Option<GlyphBitmap> {
        self.inner.rasterize_glyph(key)
    }
}

#[derive(Clone)]
struct CachedNodeLayout<L> {
    key: TextLayoutKey,
    layout: L,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TextLayoutKey {
    text: Arc<str>,
    style: TextStyleKey,
    constraints: TextLayoutConstraintsKey,
}

impl TextLayoutKey {
    fn new(text: &str, style: &ComputedTextStyle, constraints: TextLayoutConstraints) -> Self {
        Self {
            text: Arc::from(text),
            style: TextStyleKey::from(style),
            constraints: TextLayoutConstraintsKey::from(constraints),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TextLayoutConstraintsKey {
    max_width: Option<F32Key>,
}

impl From<TextLayoutConstraints> for TextLayoutConstraintsKey {
    fn from(constraints: TextLayoutConstraints) -> Self {
        Self {
            max_width: constraints.max_width.map(|width| F32Key(width.to_bits())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TextStyleKey {
    color: ColorKey,
    font_family: FontFamily,
    font_size: F32Key,
    font_weight: FontWeight,
    font_style: FontStyle,
    line_height: LineHeightKey,
    letter_spacing: F32Key,
    decoration: TextDecoration,
}

impl From<&ComputedTextStyle> for TextStyleKey {
    fn from(style: &ComputedTextStyle) -> Self {
        Self {
            color: ColorKey::from(style.color),
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
struct ColorKey {
    r: F32Key,
    g: F32Key,
    b: F32Key,
    a: F32Key,
}

impl From<Color> for ColorKey {
    fn from(color: Color) -> Self {
        Self {
            r: F32Key(color.r.to_bits()),
            g: F32Key(color.g.to_bits()),
            b: F32Key(color.b.to_bits()),
            a: F32Key(color.a.to_bits()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CosmicTextEngine;
    use xui_interface::TextStyle;

    #[test]
    fn node_layout_reuses_cached_par_for_same_key() {
        let mut text = WinitTextEngine::<Engine>::new();
        let node_id = NodeId::default();
        let style: ComputedTextStyle = TextStyle::default().into();

        let first =
            text.layout_node_text(node_id, "cached", &style, TextLayoutConstraints::UNBOUNDED);
        let second =
            text.layout_node_text(node_id, "cached", &style, TextLayoutConstraints::UNBOUNDED);

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(text.cached_layout_count(), 1);
    }

    #[test]
    fn removed_node_releases_cached_par() {
        let mut text = WinitTextEngine::<Engine>::new();
        let node_id = NodeId::default();
        let style: ComputedTextStyle = TextStyle::default().into();

        text.layout_node_text(node_id, "cached", &style, TextLayoutConstraints::UNBOUNDED);
        text.handle_node_lifecycle(&NodeLifecycleEvent::Removed(node_id));

        assert_eq!(text.cached_layout_count(), 0);
    }

    #[test]
    fn forwards_node_cache_and_lifecycle_to_inner_backend() {
        let mut text = WinitTextEngine::with_layouter(CosmicTextEngine::new());
        let node_id = NodeId::default();
        let style: ComputedTextStyle = TextStyle::default().into();

        text.layout_node_text(node_id, "cached", &style, TextLayoutConstraints::UNBOUNDED);

        assert_eq!(text.cached_layout_count(), 1);
        assert_eq!(text.inner().cached_node_count(), 1);

        text.handle_node_lifecycle(&NodeLifecycleEvent::Removed(node_id));

        assert_eq!(text.cached_layout_count(), 0);
        assert_eq!(text.inner().cached_node_count(), 0);
    }
}
