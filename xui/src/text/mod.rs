mod tree;
use std::collections::HashMap;
use std::sync::Arc;

use xui_interface::{
    Affinity, NodeId, NodeLifecycleEvent, ParagraphLayout, Point, Rect, Shaper, Size,
    TextBackend as TextBackendI, TextLayoutInput, TextOffset, TextOffsetUnit, TextPosition,
    TextRange,
};

type HeightIndex = tree::Tree<f32>;

pub(crate) enum TextHostKind<B: TextBackendI> {
    SimpleBuffer(SimpleTextLayout<B>),
    VirtualDocument(DocumentTextLayout<B>),
}

impl<B: TextBackendI> TextHostKind<B> {
    pub fn size(&self) -> Size<f32> {
        match self {
            TextHostKind::SimpleBuffer(inner) => inner.layout.size(),
            TextHostKind::VirtualDocument(_inner) => unreachable!(),
        }
    }
}

pub trait TextLayoutQuery {
    fn size(&self) -> Size<f32>;

    fn hit_test_point(&self, _point: Point) -> Option<TextPosition> {
        None
    }

    fn caret_rect(&self, _char_index: usize) -> Option<Rect> {
        None
    }

    fn selection_rects(&self, _range: TextRange) -> Vec<Rect> {
        Vec::new()
    }
}

impl<B: TextBackendI> TextLayoutQuery for TextHostKind<B> {
    fn size(&self) -> Size<f32> {
        self.size()
    }
}

struct SimpleTextLayout<B: TextBackendI> {
    layout: Arc<ParagraphLayout<<B as Shaper>::GlyphKey>>,
}

struct DocumentTextLayout<B: TextBackendI> {
    paragraphs: Vec<ParagraphCache<B>>,
    height_index: HeightIndex,
}

struct ParagraphCache<B: TextBackendI> {
    buffer: Arc<ParagraphLayout<<B as Shaper>::GlyphKey>>,
    text_rev: u64,
    style_rev: u64,
    width: f32,
    height: f32,
}

pub struct TextHost<B>
where
    B: TextBackendI,
{
    backend: B,
    node_cache: HashMap<NodeId, NodeCache<B>>,
}

pub(crate) struct NodeCache<B>
where
    B: TextBackendI,
{
    para_input: TextLayoutInput,
    pub(crate) kind: TextHostKind<B>,
    state: B::State,
}

impl<B> TextLayoutQuery for NodeCache<B>
where
    B: TextBackendI,
{
    fn size(&self) -> Size<f32> {
        self.kind.size()
    }

    fn hit_test_point(&self, point: Point) -> Option<TextPosition> {
        let layout = self.simple_layout()?;
        layout.hit_test_point(point)
    }

    fn caret_rect(&self, char_index: usize) -> Option<Rect> {
        let layout = self.simple_layout()?;
        let text = self.para_input.text.as_str();
        let offset = char_to_layout_offset(text, char_index, layout_offset_unit(layout));
        layout.caret_rect(TextPosition {
            offset,
            affinity: Affinity::After,
        })
    }

    fn selection_rects(&self, range: TextRange) -> Vec<Rect> {
        let Some(layout) = self.simple_layout() else {
            return Vec::new();
        };
        let text = self.para_input.text.as_str();
        layout.selection_rects(normalize_range_for_layout(text, layout, range))
    }
}

impl<B> NodeCache<B>
where
    B: TextBackendI,
{
    fn simple_layout(&self) -> Option<&ParagraphLayout<<B as Shaper>::GlyphKey>> {
        match &self.kind {
            TextHostKind::SimpleBuffer(inner) => Some(inner.layout.as_ref()),
            TextHostKind::VirtualDocument(_) => None,
        }
    }
}

fn layout_offset_unit<K>(layout: &ParagraphLayout<K>) -> TextOffsetUnit {
    layout
        .clusters
        .first()
        .map(|cluster| cluster.text_range.start.unit)
        .or_else(|| layout.lines.first().map(|line| line.text_range.start.unit))
        .unwrap_or(TextOffsetUnit::Char)
}

fn normalize_range_for_layout<K>(
    text: &str,
    layout: &ParagraphLayout<K>,
    range: TextRange,
) -> TextRange {
    let unit = layout_offset_unit(layout);
    TextRange::new(
        char_to_layout_offset(text, text_offset_to_char(text, range.start), unit),
        char_to_layout_offset(text, text_offset_to_char(text, range.end), unit),
    )
}

fn char_to_layout_offset(text: &str, char_index: usize, unit: TextOffsetUnit) -> TextOffset {
    let char_index = char_index.min(text.chars().count());
    match unit {
        TextOffsetUnit::Char => TextOffset::char_offset(char_index),
        TextOffsetUnit::Utf8Byte => TextOffset::byte_offset(char_to_byte_offset(text, char_index)),
        TextOffsetUnit::Utf16CodeUnit => {
            TextOffset::utf16_offset(char_to_utf16_offset(text, char_index))
        }
    }
}

fn text_offset_to_char(text: &str, offset: TextOffset) -> usize {
    match offset.unit {
        TextOffsetUnit::Char => offset.raw.min(text.chars().count()),
        TextOffsetUnit::Utf8Byte => byte_to_char_index(text, offset.raw),
        TextOffsetUnit::Utf16CodeUnit => utf16_to_char_index(text, offset.raw),
    }
}

fn char_to_byte_offset(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .map(|(byte, _)| byte)
        .nth(char_index)
        .unwrap_or(text.len())
}

fn char_to_utf16_offset(text: &str, char_index: usize) -> usize {
    text.chars().take(char_index).map(char::len_utf16).sum()
}

fn byte_to_char_index(text: &str, byte_offset: usize) -> usize {
    let byte_offset = byte_offset.min(text.len());
    text.char_indices()
        .take_while(|(byte, _)| *byte < byte_offset)
        .count()
}

fn utf16_to_char_index(text: &str, utf16_offset: usize) -> usize {
    let mut units = 0;
    for (char_index, ch) in text.chars().enumerate() {
        if units >= utf16_offset {
            return char_index;
        }
        units += ch.len_utf16();
    }
    text.chars().count()
}

impl<B> TextHost<B>
where
    B: TextBackendI,
{
    pub fn new(backend: B) -> Self {
        let cache = HashMap::new();
        Self {
            backend,
            node_cache: cache,
        }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    #[cfg(test)]
    pub(crate) fn cached_node_count(&self) -> usize {
        self.node_cache.len()
    }

    pub(crate) fn handle_node_lifecycle(&mut self, event: &NodeLifecycleEvent) {
        if let NodeLifecycleEvent::Removed(id) = event {
            self.node_cache.remove(id);
        }
        self.backend.handle_node_lifecycle(event);
    }

    pub fn simple_layout(
        &self,
        id: NodeId,
    ) -> Option<Arc<ParagraphLayout<<B as Shaper>::GlyphKey>>> {
        let cache = self.node_cache.get(&id)?;
        match &cache.kind {
            TextHostKind::SimpleBuffer(inner) => Some(inner.layout.clone()),
            TextHostKind::VirtualDocument(_) => None,
        }
    }

    pub(crate) fn get(&self, node_id: NodeId) -> Option<&NodeCache<B>> {
        self.node_cache.get(&node_id)
    }

    pub fn layout_query(&self, node_id: NodeId) -> Option<&dyn TextLayoutQuery> {
        self.node_cache
            .get(&node_id)
            .map(|cache| cache as &dyn TextLayoutQuery)
    }

    pub(crate) fn simple_doc(&mut self, id: NodeId, input: TextLayoutInput) -> &NodeCache<B> {
        if let Some(cache) = self.node_cache.get_mut(&id) {
            if cache.para_input != input {
                let layout = Arc::new(
                    self.backend
                        .layout_paragraph(&mut cache.state, input.clone()),
                );
                cache.kind = TextHostKind::SimpleBuffer(SimpleTextLayout { layout });
                cache.para_input = input;
            }
            return self
                .node_cache
                .get(&id)
                .expect("text cache entry must exist after cache hit");
        }

        let mut state = self.backend.create_state();
        let layout = Arc::new(self.backend.layout_paragraph(&mut state, input.clone()));
        self.node_cache.insert(
            id,
            NodeCache {
                kind: TextHostKind::SimpleBuffer(SimpleTextLayout { layout }),
                para_input: input,
                state,
            },
        );

        self.node_cache
            .get(&id)
            .expect("text cache entry must exist after insert")
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::sync::Arc;

    use xui_interface::{
        FontDataRef, FontDatabase, FontQuery, GlyphRasterizer, ParagraphLayout, RasterizedGlyph,
        Shaper, TextBackend, TextLayoutInput,
    };

    pub(crate) struct ZeroTextBackend;

    impl FontDatabase for ZeroTextBackend {
        type FontId = u32;

        fn epoch(&self) -> u64 {
            0
        }

        fn load_system_fonts(&mut self) {}

        fn load_font_bytes(&mut self, _bytes: Arc<[u8]>) -> Self::FontId {
            0
        }

        fn query(&self, _query: &FontQuery) -> Option<Self::FontId> {
            Some(0)
        }

        fn font_data(&self, _id: Self::FontId) -> Option<FontDataRef<'_>> {
            None
        }
    }

    impl Shaper for ZeroTextBackend {
        type State = ();
        type GlyphKey = ();

        fn create_state(&mut self) -> Self::State {}

        fn layout_paragraph(
            &mut self,
            _state: &mut Self::State,
            _input: TextLayoutInput,
        ) -> ParagraphLayout {
            ParagraphLayout {
                lines: Vec::new(),
                runs: Vec::new(),
                glyphs: Vec::new(),
                clusters: Vec::new(),
            }
        }
    }

    impl GlyphRasterizer for ZeroTextBackend {
        type GlyphKey = ();

        fn rasterize(&mut self, _key: Self::GlyphKey) -> Option<RasterizedGlyph> {
            None
        }
    }

    impl TextBackend for ZeroTextBackend {}
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use slotmap::SlotMap;
    use xui_interface::{
        FontDataRef, FontDatabase, FontQuery, GlyphFlags, GlyphInstance, GlyphRasterizer,
        LineLayout, NodeLifecycleEvent, ParagraphLayout, ParagraphStyle, RasterizedGlyph, Shaper,
        TextBackend, TextCluster, TextLayoutConstraints, TextLayoutInput, TextOffset, TextStyle,
    };

    use super::*;

    struct CountingTextBackend {
        layout_calls: usize,
        lifecycle_calls: usize,
        epoch: u64,
    }

    impl CountingTextBackend {
        fn new() -> Self {
            Self {
                layout_calls: 0,
                lifecycle_calls: 0,
                epoch: 0,
            }
        }
    }

    impl FontDatabase for CountingTextBackend {
        type FontId = u32;

        fn epoch(&self) -> u64 {
            self.epoch
        }

        fn load_system_fonts(&mut self) {
            self.epoch = self.epoch.wrapping_add(1);
        }

        fn load_font_bytes(&mut self, _bytes: Arc<[u8]>) -> Self::FontId {
            self.epoch = self.epoch.wrapping_add(1);
            0
        }

        fn query(&self, _query: &FontQuery) -> Option<Self::FontId> {
            Some(0)
        }

        fn font_data(&self, _id: Self::FontId) -> Option<FontDataRef<'_>> {
            None
        }
    }

    impl Shaper for CountingTextBackend {
        type State = ();
        type GlyphKey = ();

        fn create_state(&mut self) -> Self::State {}

        fn layout_paragraph(
            &mut self,
            _state: &mut Self::State,
            _input: TextLayoutInput,
        ) -> ParagraphLayout {
            self.layout_calls += 1;
            ParagraphLayout {
                lines: Vec::new(),
                runs: Vec::new(),
                glyphs: Vec::new(),
                clusters: Vec::new(),
            }
        }

        fn handle_node_lifecycle(&mut self, _event: &NodeLifecycleEvent) {
            self.lifecycle_calls += 1;
        }
    }

    impl GlyphRasterizer for CountingTextBackend {
        type GlyphKey = ();

        fn rasterize(&mut self, _key: Self::GlyphKey) -> Option<RasterizedGlyph> {
            None
        }
    }

    impl TextBackend for CountingTextBackend {}

    #[derive(Default)]
    struct FixedTextBackend;

    impl FontDatabase for FixedTextBackend {
        type FontId = u32;

        fn epoch(&self) -> u64 {
            0
        }

        fn load_system_fonts(&mut self) {}

        fn load_font_bytes(&mut self, _bytes: Arc<[u8]>) -> Self::FontId {
            0
        }

        fn query(&self, _query: &FontQuery) -> Option<Self::FontId> {
            Some(0)
        }

        fn font_data(&self, _id: Self::FontId) -> Option<FontDataRef<'_>> {
            None
        }
    }

    impl Shaper for FixedTextBackend {
        type State = ();
        type GlyphKey = ();

        fn create_state(&mut self) -> Self::State {}

        fn layout_paragraph(
            &mut self,
            _state: &mut Self::State,
            input: TextLayoutInput,
        ) -> ParagraphLayout {
            let text = input.text.as_str();
            let mut glyphs = Vec::new();
            let mut clusters = Vec::new();
            let mut chars = text.char_indices().peekable();

            while let Some((byte_start, _)) = chars.next() {
                let char_index = glyphs.len();
                let byte_end = chars
                    .peek()
                    .map(|(next_byte, _)| *next_byte)
                    .unwrap_or(text.len());
                let x = char_index as f32 * 10.0;
                glyphs.push(GlyphInstance {
                    key: (),
                    glyph_id: char_index as u32,
                    draw_pos: Point::new(x, 0.0),
                    hitbox: Rect::new(x, 0.0, 10.0, 20.0),
                    cluster: char_index,
                    flags: GlyphFlags::empty(),
                });
                clusters.push(TextCluster {
                    source_line: 0,
                    local_text_range: byte_start..byte_end,
                    text_range: TextRange::new(
                        TextOffset::byte_offset(byte_start),
                        TextOffset::byte_offset(byte_end),
                    ),
                    glyph_range: char_index..char_index + 1,
                    hitbox: Rect::new(x, 0.0, 10.0, 20.0),
                });
            }

            let char_len = glyphs.len();
            ParagraphLayout {
                lines: vec![LineLayout {
                    source_line: 0,
                    text_range: TextRange::new(
                        TextOffset::byte_offset(0),
                        TextOffset::byte_offset(text.len()),
                    ),
                    run_range: 0..0,
                    glyph_range: 0..char_len,
                    cluster_range: 0..char_len,
                    x: 0.0,
                    y: 0.0,
                    width: char_len as f32 * 10.0,
                    height: 20.0,
                    baseline: 16.0,
                    hard_break: false,
                    ellipsized: false,
                }],
                runs: Vec::new(),
                glyphs,
                clusters,
            }
        }
    }

    impl GlyphRasterizer for FixedTextBackend {
        type GlyphKey = ();

        fn rasterize(&mut self, _key: Self::GlyphKey) -> Option<RasterizedGlyph> {
            None
        }
    }

    impl TextBackend for FixedTextBackend {}

    fn node_id() -> NodeId {
        let mut nodes = SlotMap::<NodeId, ()>::with_key();
        nodes.insert(())
    }

    fn two_node_ids() -> (NodeId, NodeId) {
        let mut nodes = SlotMap::<NodeId, ()>::with_key();
        (nodes.insert(()), nodes.insert(()))
    }

    fn input(text: &'static str) -> TextLayoutInput {
        TextLayoutInput::new(
            text.into(),
            TextLayoutConstraints::UNBOUNDED,
            TextStyle::default().into(),
            ParagraphStyle::default(),
            0,
        )
    }

    #[test]
    fn simple_doc_reuses_layout_for_identical_input() {
        let id = node_id();
        let mut host = TextHost::new(CountingTextBackend::new());

        host.simple_doc(id, input("cached"));
        host.simple_doc(id, input("cached"));

        assert_eq!(host.backend().layout_calls, 1);
        assert_eq!(host.cached_node_count(), 1);
    }

    #[test]
    fn simple_doc_relayouts_when_input_changes() {
        let id = node_id();
        let mut host = TextHost::new(CountingTextBackend::new());

        host.simple_doc(id, input("cached"));
        host.simple_doc(id, input("changed"));

        let mut constrained = input("changed");
        constrained.constraints = TextLayoutConstraints::max_width(42.0);
        host.simple_doc(id, constrained);

        let mut restyled = input("changed");
        restyled.default_style.font_size = 20.0;
        host.simple_doc(id, restyled);

        let mut font_changed = input("changed");
        font_changed.font_context_revision = 1;
        host.simple_doc(id, font_changed);

        assert_eq!(host.backend().layout_calls, 5);
        assert_eq!(host.cached_node_count(), 1);
    }

    #[test]
    fn simple_doc_caches_each_node_separately() {
        let (first, second) = two_node_ids();
        let mut host = TextHost::new(CountingTextBackend::new());

        host.simple_doc(first, input("cached"));
        host.simple_doc(second, input("cached"));
        host.simple_doc(first, input("cached"));
        host.simple_doc(second, input("cached"));

        assert_eq!(host.backend().layout_calls, 2);
        assert_eq!(host.cached_node_count(), 2);
    }

    #[test]
    fn removed_node_releases_text_host_cache_and_forwards_lifecycle() {
        let id = node_id();
        let mut host = TextHost::new(CountingTextBackend::new());

        host.simple_doc(id, input("cached"));
        host.handle_node_lifecycle(&NodeLifecycleEvent::Removed(id));

        assert_eq!(host.cached_node_count(), 0);
        assert_eq!(host.backend().lifecycle_calls, 1);

        host.simple_doc(id, input("cached"));

        assert_eq!(host.backend().layout_calls, 2);
        assert_eq!(host.cached_node_count(), 1);
    }
}
