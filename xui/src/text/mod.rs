mod tree;
use std::collections::HashMap;
use std::sync::Arc;

use xui_interface::{
    NodeId, NodeLifecycleEvent, ParagraphLayout, Shaper, Size, TextBackend as TextBackendI,
    TextLayoutInput,
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
        FontDataRef, FontDatabase, FontQuery, GlyphRasterizer, NodeLifecycleEvent, ParagraphLayout,
        ParagraphStyle, RasterizedGlyph, Shaper, TextBackend, TextLayoutConstraints,
        TextLayoutInput, TextStyle,
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
