use std::collections::HashMap;
use std::sync::Arc;

use xui_interface::{
    Color, ComputedTextStyle, FontFamily, FontStyle, FontWeight, LineHeight, NodeId,
    NodeLifecycleEvent, Size, TextDecoration, TextLayoutConstraints, TextMeasurer,
};
use xui_text::engine::{Engine, TextLayouter};
use xui_text::par::Par;

pub struct WinitTextEngine<T = Engine> {
    inner: T,
    layout_cache: HashMap<NodeId, CachedNodeLayout>,
}

impl WinitTextEngine<Engine> {
    pub fn new() -> Self {
        Self::with_layouter(Engine::new())
    }
}

impl<T> WinitTextEngine<T> {
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

impl<T: TextLayouter> TextMeasurer for WinitTextEngine<T> {
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
        let par = self.layout_node_text(node_id, text, props, constraints);
        size_for_par(&par)
    }

    fn handle_node_lifecycle(&mut self, event: &NodeLifecycleEvent) {
        if let NodeLifecycleEvent::Removed(id) = event {
            self.layout_cache.remove(id);
        }
    }
}

impl<T: TextLayouter> TextLayouter for WinitTextEngine<T> {
    fn layout_text(
        &mut self,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Arc<Par> {
        self.inner.layout_text(text, style, constraints)
    }

    fn layout_node_text(
        &mut self,
        node_id: NodeId,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Arc<Par> {
        let key = TextLayoutKey::new(text, style, constraints);
        if let Some(cached) = self.layout_cache.get(&node_id) {
            if cached.key == key {
                return Arc::clone(&cached.par);
            }
        }

        let par = self.inner.layout_text(text, style, constraints);
        self.layout_cache.insert(
            node_id,
            CachedNodeLayout {
                key,
                par: Arc::clone(&par),
            },
        );
        par
    }

    fn get_cached_layout(&self, node_id: NodeId) -> Option<Arc<Par>> {
        self.layout_cache
            .get(&node_id)
            .map(|cached| Arc::clone(&cached.par))
    }
}

#[derive(Clone)]
struct CachedNodeLayout {
    key: TextLayoutKey,
    par: Arc<Par>,
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

fn size_for_par(par: &Par) -> Size<f32> {
    let mut width: f32 = 0.0;
    let mut height = 0.0;
    for line in par.lines() {
        width = width.max(line.advance_without_trailing_whitespace());
        height += line.size();
    }

    Size::<f32>::new(width, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xui_interface::TextStyle;

    #[test]
    fn node_layout_reuses_cached_par_for_same_key() {
        let mut text = WinitTextEngine::new();
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
        let mut text = WinitTextEngine::new();
        let node_id = NodeId::default();
        let style: ComputedTextStyle = TextStyle::default().into();

        text.layout_node_text(node_id, "cached", &style, TextLayoutConstraints::UNBOUNDED);
        text.handle_node_lifecycle(&NodeLifecycleEvent::Removed(node_id));

        assert_eq!(text.cached_layout_count(), 0);
    }
}
