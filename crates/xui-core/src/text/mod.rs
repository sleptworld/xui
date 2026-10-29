pub mod cache;
mod sequence;

pub use cache::{
    ParagraphId, ParagraphInfo, ShaperState, TextCacheStats, TextDocumentId, TextHost,
    TextLayoutHandle, TextLayoutSlot, TextUnitId, TextUnitLocation, TextUnitRef,
};
use xui_interface::{Affinity, NodeId, Point, Rect, Size, TextBackend, TextPosition, TextRange};

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

    /// [`hit_test_point`](Self::hit_test_point) as a byte offset into the
    /// laid-out text, whatever unit the backend reports in.
    fn hit_test_byte(&self, _point: Point) -> Option<(usize, Affinity)> {
        None
    }

    /// The caret before byte `offset` of the laid-out text.
    fn caret_rect_at_byte(&self, _offset: usize, _affinity: Affinity) -> Option<Rect> {
        None
    }

    /// The lines' boxes, top to bottom, in layout coordinates.
    fn line_rects(&self) -> Vec<Rect> {
        Vec::new()
    }
}

/// Read access to every resident layout and virtual document of every owner,
/// for widgets that own more than one text unit. Events reach those widgets
/// with one (`EventContext::text_units`).
pub trait TextUnitLookup {
    fn active_query(&self, owner: NodeId, unit: TextUnitRef) -> Option<&dyn TextLayoutQuery>;
    fn document_len(&self, owner: NodeId, document: TextDocumentId) -> Option<usize>;
    fn document_height(&self, owner: NodeId, document: TextDocumentId) -> Option<f32>;
    fn paragraph_at(&self, owner: NodeId, document: TextDocumentId, index: usize)
    -> Option<ParagraphId>;
    fn paragraph_at_y(&self, owner: NodeId, document: TextDocumentId, y: f32)
    -> Option<ParagraphId>;
    fn paragraph_index(
        &self,
        owner: NodeId,
        document: TextDocumentId,
        paragraph: ParagraphId,
    ) -> Option<usize>;
    fn paragraph_top(
        &self,
        owner: NodeId,
        document: TextDocumentId,
        paragraph: ParagraphId,
    ) -> Option<f32>;
    fn paragraph_height(
        &self,
        owner: NodeId,
        document: TextDocumentId,
        paragraph: ParagraphId,
    ) -> Option<f32>;
}

impl<B: TextBackend> TextUnitLookup for TextHost<B> {
    fn active_query(&self, owner: NodeId, unit: TextUnitRef) -> Option<&dyn TextLayoutQuery> {
        self.query(self.active_layout(owner, unit)?)
    }

    fn document_len(&self, owner: NodeId, document: TextDocumentId) -> Option<usize> {
        TextHost::document_len(self, owner, document)
    }

    fn document_height(&self, owner: NodeId, document: TextDocumentId) -> Option<f32> {
        TextHost::document_height(self, owner, document)
    }

    fn paragraph_at(
        &self,
        owner: NodeId,
        document: TextDocumentId,
        index: usize,
    ) -> Option<ParagraphId> {
        TextHost::paragraph_at(self, owner, document, index).map(|info| info.id)
    }

    fn paragraph_at_y(
        &self,
        owner: NodeId,
        document: TextDocumentId,
        y: f32,
    ) -> Option<ParagraphId> {
        TextHost::paragraph_at_y(self, owner, document, y).map(|info| info.id)
    }

    fn paragraph_index(
        &self,
        owner: NodeId,
        document: TextDocumentId,
        paragraph: ParagraphId,
    ) -> Option<usize> {
        TextHost::paragraph_index(self, owner, document, paragraph)
    }

    fn paragraph_top(
        &self,
        owner: NodeId,
        document: TextDocumentId,
        paragraph: ParagraphId,
    ) -> Option<f32> {
        TextHost::paragraph_top(self, owner, document, paragraph)
    }

    fn paragraph_height(
        &self,
        owner: NodeId,
        document: TextDocumentId,
        paragraph: ParagraphId,
    ) -> Option<f32> {
        TextHost::paragraph(self, owner, document, paragraph).map(|info| info.height())
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::sync::{Arc, LazyLock};

    use xui_interface::{
        FontQuery, NoFonts, NoRasterizer, ParagraphLayout, Shaper, TextBackend, TextLayoutInput,
    };

    /// Lays every paragraph out empty, without fonts.
    pub(crate) struct ZeroTextBackend;

    static NO_FONTS: LazyLock<Arc<NoFonts<u32>>> = LazyLock::new(Arc::default);

    impl Shaper for ZeroTextBackend {
        type FontId = u32;
        type State = ();

        fn create_state(&mut self) -> Self::State {}

        fn layout_paragraph(
            &mut self,
            _state: &mut Self::State,
            _input: TextLayoutInput,
        ) -> ParagraphLayout<u32> {
            ParagraphLayout {
                lines: Vec::new(),
                runs: Vec::new(),
                glyphs: Vec::new(),
                clusters: Vec::new(),
            }
        }
    }

    impl TextBackend for ZeroTextBackend {
        type FontId = u32;
        type Fonts = NoFonts<u32>;
        type Shaper = Self;
        type Rasterizer = NoRasterizer<u32>;

        fn fonts(&self) -> &Arc<NoFonts<u32>> {
            &NO_FONTS
        }

        fn shaper(&mut self) -> &mut Self {
            self
        }

        fn rasterizer(&self) -> Option<&Arc<NoRasterizer<u32>>> {
            None
        }

        fn load_system_fonts(&mut self) {}

        fn load_font_bytes(&mut self, _bytes: Arc<[u8]>) -> Option<u32> {
            None
        }

        fn query(&mut self, _query: &FontQuery) -> Option<u32> {
            Some(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use slotmap::SlotMap;
    use xui_interface::{
        FontQuery, NoFonts, NoRasterizer, NodeId, NodeLifecycleEvent, ParagraphLayout,
        ParagraphStyle, Shaper, TextBackend, TextBoxStyle, TextLayoutConstraints, TextLayoutInput,
        TextStyle,
    };

    use super::*;

    struct CountingTextBackend {
        create_state_calls: usize,
        layout_calls: usize,
        lifecycle_calls: usize,
        fonts: Arc<NoFonts<u32>>,
    }

    impl CountingTextBackend {
        fn new() -> Self {
            Self {
                create_state_calls: 0,
                layout_calls: 0,
                lifecycle_calls: 0,
                fonts: Arc::default(),
            }
        }
    }

    #[derive(Default)]
    struct CountingParagraphState {
        layout_calls: usize,
    }

    impl Shaper for CountingTextBackend {
        type FontId = u32;
        type State = CountingParagraphState;

        fn create_state(&mut self) -> Self::State {
            self.create_state_calls += 1;
            CountingParagraphState::default()
        }

        fn layout_paragraph(
            &mut self,
            state: &mut Self::State,
            _input: TextLayoutInput,
        ) -> ParagraphLayout<u32> {
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

    impl TextBackend for CountingTextBackend {
        type FontId = u32;
        type Fonts = NoFonts<u32>;
        type Shaper = Self;
        type Rasterizer = NoRasterizer<u32>;

        fn fonts(&self) -> &Arc<NoFonts<u32>> {
            &self.fonts
        }

        fn shaper(&mut self) -> &mut Self {
            self
        }

        fn rasterizer(&self) -> Option<&Arc<NoRasterizer<u32>>> {
            None
        }

        fn load_system_fonts(&mut self) {
            self.fonts.bump_epoch();
        }

        fn load_font_bytes(&mut self, _bytes: Arc<[u8]>) -> Option<u32> {
            self.fonts.bump_epoch();
            Some(0)
        }

        fn query(&mut self, _query: &FontQuery) -> Option<u32> {
            Some(0)
        }
    }

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

    /// Layout probes a paragraph under several constraints and then paints
    /// it at the last one: the paint reuses that layout, the paragraph keeps
    /// one layout in all, and probing again shapes nothing.
    #[test]
    fn probes_are_remembered_and_only_the_painted_layout_is_kept() {
        let mut owners = SlotMap::<NodeId, ()>::with_key();
        let owner = owners.insert(());
        let mut host = TextHost::new(CountingTextBackend::new());
        let slot = TextLayoutSlot::PRIMARY;
        let probes = [
            TextLayoutConstraints::MIN_SIZE,
            TextLayoutConstraints::UNBOUNDED,
            TextLayoutConstraints::max_width(80.0),
        ];

        for constraints in probes {
            host.measure_slot(owner, slot, input(constraints));
        }
        assert_eq!(host.backend().layout_calls, 3);

        host.activate_slot(owner, slot, input(TextLayoutConstraints::max_width(80.0)));
        assert_eq!(host.backend().layout_calls, 3, "painting shaped the final width again");
        assert_eq!(host.stats().layouts, 1, "only the painted layout stays");

        for constraints in probes {
            host.measure_slot(owner, slot, input(constraints));
        }
        assert_eq!(host.backend().layout_calls, 3, "a probe was shaped again");
    }

    /// A text input is measured and painted unbounded: one shaping.
    #[test]
    fn a_paragraph_painted_as_measured_is_shaped_once() {
        let mut owners = SlotMap::<NodeId, ()>::with_key();
        let owner = owners.insert(());
        let mut host = TextHost::new(CountingTextBackend::new());
        let slot = TextLayoutSlot::PRIMARY;

        host.measure_slot(owner, slot, input(TextLayoutConstraints::UNBOUNDED));
        host.activate_slot(owner, slot, input(TextLayoutConstraints::UNBOUNDED));
        assert_eq!(host.backend().layout_calls, 1);
        assert_eq!(host.stats().layouts, 1);
    }

    #[test]
    fn new_content_drops_the_variants_of_the_old_but_not_the_active_one() {
        let mut owners = SlotMap::<NodeId, ()>::with_key();
        let owner = owners.insert(());
        let mut host = TextHost::new(CountingTextBackend::new());
        let slot = TextLayoutSlot::PRIMARY;
        let other = |constraints| {
            TextLayoutInput::new(
                "other".into(),
                constraints,
                TextStyle::default().into(),
                ParagraphStyle::default(),
                TextBoxStyle::default(),
                0,
            )
        };

        let active = host.activate_slot(owner, slot, input(TextLayoutConstraints::max_width(120.0)));
        host.measure_slot(owner, slot, input(TextLayoutConstraints::UNBOUNDED));
        host.measure_slot(owner, slot, input(TextLayoutConstraints::UNBOUNDED));
        assert_eq!(host.backend().layout_calls, 2, "a measured variant is kept");

        host.measure_slot(owner, slot, other(TextLayoutConstraints::UNBOUNDED));
        assert_eq!(host.backend().layout_calls, 3);
        assert_eq!(
            host.active_slot(owner, slot),
            Some(active),
            "the variant on screen went with the old content"
        );

        host.measure_slot(owner, slot, input(TextLayoutConstraints::UNBOUNDED));
        assert_eq!(
            host.backend().layout_calls,
            4,
            "a variant of the old content outlived the new content"
        );
    }

    #[test]
    fn removing_an_owner_removes_its_units_and_no_others() {
        let mut owners = SlotMap::<NodeId, ()>::with_key();
        let (gone, kept) = (owners.insert(()), owners.insert(()));
        let mut host = TextHost::new(CountingTextBackend::new());

        let measure = |host: &mut TextHost<CountingTextBackend>, owner, slot| {
            host.measure_slot(owner, slot, input(TextLayoutConstraints::UNBOUNDED));
            host.direct_unit(owner, slot).unwrap()
        };
        let gone_slots = [
            measure(&mut host, gone, TextLayoutSlot::PRIMARY),
            measure(&mut host, gone, TextLayoutSlot::new(1)),
        ];
        let kept_slot = measure(&mut host, kept, TextLayoutSlot::PRIMARY);
        let range = xui_interface::TextRange::new(
            xui_interface::TextOffset::char_offset(0),
            xui_interface::TextOffset::char_offset(4),
        );
        let gone_document = host.create_document(gone);
        let gone_paragraph = host
            .insert_paragraph(gone, gone_document, 0, range, 0, 0, 10.0)
            .unwrap()
            .unit;
        let kept_document = host.create_document(kept);
        let kept_paragraph = host
            .insert_paragraph(kept, kept_document, 0, range, 0, 0, 10.0)
            .unwrap()
            .unit;

        host.handle_node_lifecycle(&NodeLifecycleEvent::Removed(gone));

        for unit in gone_slots.into_iter().chain([gone_paragraph]) {
            assert!(host.unit_location(unit).is_none(), "a unit of the removed owner is left");
        }
        for unit in [kept_slot, kept_paragraph] {
            assert!(host.unit_location(unit).is_some(), "another owner's unit was removed");
        }
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
