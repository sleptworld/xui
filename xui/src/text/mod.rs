pub mod cache;

pub use cache::{
    ParagraphId, ParagraphInfo, TextCacheStats, TextDocumentId, TextHost, TextLayoutHandle,
    TextLayoutSlot, TextUnitId, TextUnitLocation,
};
use xui_interface::{Point, Rect, Size, TextPosition, TextRange};

/// Geometry queries supported by a resident shaped-text layout.
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
        FontDataRef, FontDatabase, FontQuery, GlyphRasterizer, NodeId, NodeLifecycleEvent,
        ParagraphLayout, ParagraphStyle, RasterizedGlyph, Shaper, TextBackend,
        TextLayoutConstraints, TextLayoutInput, TextStyle,
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

    fn input(text: &'static str) -> TextLayoutInput {
        TextLayoutInput::new(
            text.into(),
            TextLayoutConstraints::UNBOUNDED,
            TextStyle::default().into(),
            ParagraphStyle::default(),
            xui_interface::TextBoxStyle::default(),
            0,
        )
    }

    #[test]
    fn get_or_shape_slot_reuses_identical_input() {
        let owner = node_id();
        let mut host = TextHost::new(CountingTextBackend::new());

        let first = host.get_or_shape_slot(owner, TextLayoutSlot::PRIMARY, input("cached"));
        let second = host.get_or_shape_slot(owner, TextLayoutSlot::PRIMARY, input("cached"));

        assert_eq!(first, second);
        assert_eq!(host.backend().layout_calls, 1);
        assert_eq!(host.stats().owners, 1);
        assert_eq!(host.stats().units, 1);
    }

    #[test]
    fn changed_input_shapes_a_new_variant() {
        let owner = node_id();
        let mut host = TextHost::new(CountingTextBackend::new());

        let first = host.get_or_shape_slot(owner, TextLayoutSlot::PRIMARY, input("cached"));
        let second = host.get_or_shape_slot(owner, TextLayoutSlot::PRIMARY, input("changed"));

        assert_ne!(first, second);
        assert_eq!(host.backend().layout_calls, 2);
        assert_eq!(
            host.active_slot(owner, TextLayoutSlot::PRIMARY),
            Some(second)
        );
    }

    #[test]
    fn paint_only_text_style_changes_reuse_the_shaped_variant() {
        let owner = node_id();
        let mut host = TextHost::new(CountingTextBackend::new());
        let original = input("cached");
        let first = host.get_or_shape_slot(owner, TextLayoutSlot::PRIMARY, original.clone());

        let mut recolored = original.clone();
        recolored.default_style.color = xui_interface::Color::WHITE;
        recolored.default_style.decoration.underline = true;
        let second = host.get_or_shape_slot(owner, TextLayoutSlot::PRIMARY, recolored);

        assert_eq!(first, second);
        assert_eq!(host.backend().layout_calls, 1);

        let mut resized = original;
        resized.default_style.font_size += 1.0;
        let third = host.get_or_shape_slot(owner, TextLayoutSlot::PRIMARY, resized);
        assert_ne!(second, third);
        assert_eq!(host.backend().layout_calls, 2);
    }

    #[test]
    fn removed_owner_releases_layouts_and_forwards_lifecycle() {
        let owner = node_id();
        let mut host = TextHost::new(CountingTextBackend::new());
        host.get_or_shape_slot(owner, TextLayoutSlot::PRIMARY, input("cached"));

        host.handle_node_lifecycle(&NodeLifecycleEvent::Removed(owner));

        assert_eq!(host.stats().owners, 0);
        assert_eq!(host.stats().layouts, 0);
        assert_eq!(host.backend().lifecycle_calls, 1);
    }
}
