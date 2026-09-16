pub mod cache;

pub use cache::{
    ParagraphId, ParagraphInfo, TextCacheStats, TextDocumentId, TextHost, TextLayoutHandle,
    TextLayoutSlot, TextUnitId, TextUnitLocation,
};
use xui_interface::{Point, Rect, Size, TextPosition, TextRange};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TextLayoutMetrics {
    pub size: Size<f32>,
    pub first_baseline: Option<f32>,
    pub line_count: usize,
}

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

        fn query(&mut self, _query: &FontQuery) -> Option<Self::FontId> {
            Some(0)
        }

        fn font_data(&self, _id: Self::FontId) -> Option<FontDataRef<'_>> {
            None
        }
    }

    impl Shaper for ZeroTextBackend {
        type State = ();
        type GlyphKey = ();
        type FontId = u32;

        fn create_state(&mut self) -> Self::State {}

        fn layout_paragraph(
            &mut self,
            _state: &mut Self::State,
            _input: TextLayoutInput,
        ) -> ParagraphLayout<Self::FontId, Self::GlyphKey> {
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
        ParagraphLayout, ParagraphStyle, RasterizedGlyph, Shaper, TextBackend, TextBoxStyle,
        TextLayoutConstraints, TextLayoutInput, TextStyle,
    };

    use super::*;

    struct CountingTextBackend {
        create_state_calls: usize,
        layout_calls: usize,
        lifecycle_calls: usize,
        epoch: u64,
    }

    impl CountingTextBackend {
        fn new() -> Self {
            Self {
                create_state_calls: 0,
                layout_calls: 0,
                lifecycle_calls: 0,
                epoch: 0,
            }
        }
    }

    #[derive(Default)]
    struct CountingParagraphState {
        layout_calls: usize,
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

        fn query(&mut self, _query: &FontQuery) -> Option<Self::FontId> {
            Some(0)
        }

        fn font_data(&self, _id: Self::FontId) -> Option<FontDataRef<'_>> {
            None
        }
    }

    impl Shaper for CountingTextBackend {
        type State = CountingParagraphState;
        type GlyphKey = ();
        type FontId = u32;

        fn create_state(&mut self) -> Self::State {
            self.create_state_calls += 1;
            CountingParagraphState::default()
        }

        fn layout_paragraph(
            &mut self,
            state: &mut Self::State,
            _input: TextLayoutInput,
        ) -> ParagraphLayout<Self::FontId, Self::GlyphKey> {
            self.layout_calls += 1;
            state.layout_calls += 1;
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

    fn input(constraints: TextLayoutConstraints) -> TextLayoutInput {
        TextLayoutInput::new(
            "tabs".into(),
            constraints,
            TextStyle::default().into(),
            ParagraphStyle::default(),
            TextBoxStyle::default(),
            0,
        )
    }

    #[test]
    fn measurement_does_not_replace_the_active_variant() {
        let mut owners = SlotMap::<NodeId, ()>::with_key();
        let owner = owners.insert(());
        let mut host = TextHost::new(CountingTextBackend::new());

        let active = host.activate_slot(
            owner,
            TextLayoutSlot::PRIMARY,
            input(TextLayoutConstraints::max_width(120.0)),
        );
        assert_eq!(
            host.active_slot(owner, TextLayoutSlot::PRIMARY),
            Some(active)
        );

        host.measure_slot(
            owner,
            TextLayoutSlot::PRIMARY,
            input(TextLayoutConstraints::MIN_SIZE),
        );

        assert_eq!(host.backend().layout_calls, 2);
        assert_eq!(host.backend().create_state_calls, 1);
        assert_eq!(host.state(active).unwrap().layout_calls, 2);
        assert_eq!(
            host.active_slot(owner, TextLayoutSlot::PRIMARY),
            Some(active)
        );
    }

    #[test]
    fn shaper_state_lives_for_the_text_unit() {
        let mut owners = SlotMap::<NodeId, ()>::with_key();
        let owner = owners.insert(());
        let mut host = TextHost::new(CountingTextBackend::new());

        host.measure_slot(
            owner,
            TextLayoutSlot::PRIMARY,
            input(TextLayoutConstraints::UNBOUNDED),
        );
        host.measure_slot(
            owner,
            TextLayoutSlot::PRIMARY,
            input(TextLayoutConstraints::max_width(80.0)),
        );
        assert_eq!(host.backend().create_state_calls, 1);

        host.invalidate_slot(owner, TextLayoutSlot::PRIMARY);
        let active = host.activate_slot(
            owner,
            TextLayoutSlot::PRIMARY,
            input(TextLayoutConstraints::max_width(40.0)),
        );
        assert_eq!(host.backend().create_state_calls, 1);
        assert_eq!(host.state(active).unwrap().layout_calls, 3);

        host.measure_slot(
            owner,
            TextLayoutSlot::new(1),
            input(TextLayoutConstraints::UNBOUNDED),
        );
        assert_eq!(host.backend().create_state_calls, 2);
    }
}
