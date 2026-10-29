use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use xui_f::FBackend;
use xui_interface::events::{
    PointerButton, PointerButtons, PointerKind, RawEvent, RawPointerButton, RawWheel, ScrollDelta,
    XuiPointerId,
};
use xui_interface::{
    Bounds, Color, EventResult, Modifiers, NodeId, Point, Size, TextContent, Translation,
    WidgetUpdateFlags,
};

use super::*;
use crate::event_system::translator::{EventTranslator, EventTranslatorConfig};
use crate::text::TextHost;
use crate::ui_runtime::UiRuntime;
use crate::widgets::Widgets;

/// One block per line of text, with ids that survive edits.
#[derive(Default)]
struct Lines {
    lines: Vec<(BlockId, String)>,
    next_id: u64,
    changes: Vec<BlockChange>,
    reset: bool,
    decorations: Vec<SurfaceDecor>,
    caret: Option<SurfacePosition>,
    pointers: Vec<SurfacePointer>,
    typed: Vec<String>,
}

impl Lines {
    fn new(count: usize) -> Self {
        let mut lines = Self::default();
        for index in 0..count {
            lines.push(format!("line {index}"));
        }
        lines.reset = true;
        lines
    }

    fn push(&mut self, text: String) -> BlockId {
        let id = BlockId(self.next_id);
        self.next_id += 1;
        self.lines.push((id, text));
        id
    }

    fn insert(&mut self, index: usize, text: &str) -> BlockId {
        let id = BlockId(self.next_id);
        self.next_id += 1;
        self.lines.insert(index, (id, text.to_owned()));
        self.changes.push(BlockChange::Inserted { index, id });
        id
    }

    fn remove(&mut self, index: usize) {
        let (id, _) = self.lines.remove(index);
        self.changes.push(BlockChange::Removed { id });
    }

    fn id(&self, index: usize) -> BlockId {
        self.lines[index].0
    }
}

impl TextSurfaceModel for Lines {
    fn take_changes(&mut self) -> SurfaceChanges {
        if std::mem::take(&mut self.reset) {
            self.changes.clear();
            return SurfaceChanges::Reset;
        }
        let changes = std::mem::take(&mut self.changes);
        if changes.is_empty() {
            SurfaceChanges::None
        } else {
            SurfaceChanges::Edits(changes)
        }
    }

    fn block_count(&self) -> usize {
        self.lines.len()
    }

    fn block_at(&self, index: usize) -> BlockId {
        self.lines[index].0
    }

    fn block(&self, id: BlockId) -> SurfaceBlock {
        let text = self
            .lines
            .iter()
            .find(|(line, _)| *line == id)
            .map_or_else(String::new, |(_, text)| text.clone());
        SurfaceBlock {
            text: TextContent::from(text),
            ..SurfaceBlock::default()
        }
    }

    fn decorations(&self, _visible: &[BlockId]) -> Vec<SurfaceDecor> {
        self.decorations.clone()
    }

    fn primary_caret(&self) -> Option<SurfacePosition> {
        self.caret
    }

    fn handle_input(&mut self, input: SurfaceInput<'_>, cx: &mut SurfaceCx<'_, '_>) -> EventResult {
        match input {
            SurfaceInput::Pointer(pointer) => {
                self.pointers.push(pointer);
                if let Some(at) = pointer.at {
                    self.caret = Some(at);
                    cx.changed();
                }
                EventResult::Consumed
            }
            SurfaceInput::Text(text) => {
                self.typed.push(text.to_owned());
                EventResult::Consumed
            }
            _ => EventResult::Ignored,
        }
    }
}

struct Fixture {
    arena: UiRuntime,
    text: TextHost<FBackend>,
    model: Rc<RefCell<Lines>>,
    handle: TextSurfaceHandle,
    surface: NodeId,
}

const VIEWPORT: Size<f32> = Size {
    width: 400.0,
    height: 300.0,
};

fn fixture(lines: usize, wrap: SurfaceWrap) -> Option<Fixture> {
    let backend = FBackend::new();
    if backend.face_count() == 0 {
        return None;
    }
    let model = Rc::new(RefCell::new(Lines::new(lines)));
    let handle = TextSurfaceHandle::new(model.clone());
    let mut arena = UiRuntime::new();
    let surface = arena.create_node(Box::new(
        text_surface()
            .with::<text_surface::model>(handle.clone())
            .with::<text_surface::wrap>(wrap)
            .with::<text_surface::size>(Size::fix(VIEWPORT.width, VIEWPORT.height)),
    ));
    let root = arena.root();
    arena.place(root, surface, None);
    let mut fixture = Fixture {
        arena,
        text: TextHost::new(backend),
        model,
        handle,
        surface,
    };
    fixture.frame();
    Some(fixture)
}

impl Fixture {
    fn frame(&mut self) {
        self.arena
            .update_tree(Size::new(800.0, 600.0), &mut self.text);
    }

    fn state<R>(&self, read: impl FnOnce(&TextSurfaceState) -> R) -> R {
        self.arena
            .node_view(self.surface)
            .widget
            .with_widgets(|widget| match widget {
                Widgets::TextSurface(surface) => read(&surface.state),
                _ => panic!("not a text surface"),
            })
    }

    fn edit_state(&mut self, edit: impl FnOnce(&mut TextSurfaceState)) {
        self.arena
            .node_view(self.surface)
            .widget
            .with_widgets_mut(|widget| match widget {
                Widgets::TextSurface(surface) => edit(&mut surface.state),
                _ => panic!("not a text surface"),
            });
        self.arena
            .request_update(self.surface, WidgetUpdateFlags::TEXT_SHAPE);
    }

    fn document(&self) -> TextDocumentId {
        self.state(|state| state.document.expect("synced"))
    }

    /// Where `index`'s block is painted, relative to the viewport's top.
    fn screen_y(&self, block: BlockId) -> Option<f32> {
        let paragraph = self.state(|state| state.paragraph_of(block))?;
        let scroll = self.state(|state| state.scroll_offset());
        self.state(|state| state.painted_blocks())
            .into_iter()
            .find(|(painted, _)| *painted == paragraph)
            .map(|(_, top)| top - scroll.y)
    }

    fn dispatch(&mut self, event: RawEvent) -> EventResult {
        let mut translator = EventTranslator::new(EventTranslatorConfig::default());
        self.arena
            .dispatch_event(&self.text, &mut translator, event)
    }

    fn bounds(&self) -> Bounds {
        self.arena.node(self.surface).unwrap().layout
    }
}

fn pointer(position: Point) -> RawPointerButton {
    RawPointerButton {
        position,
        pointer_id: XuiPointerId::new(0),
        device_id: None,
        kind: PointerKind::Mouse,
        button: PointerButton::Primary,
        buttons: PointerButtons::default(),
        modifiers: Modifiers::default(),
        timestamp: Instant::now(),
    }
}

fn wheel(position: Point, dy: f32) -> RawEvent {
    RawEvent::Wheel(RawWheel {
        position,
        delta: ScrollDelta::Pixels(Translation::new(0.0, dy)),
        device_id: None,
        pointer_id: None,
        modifiers: Modifiers::default(),
        timestamp: Instant::now(),
        is_inertial: false,
    })
}

#[test]
fn only_blocks_near_the_viewport_are_laid_out() {
    let Some(fixture) = fixture(10_000, SurfaceWrap::Width) else {
        return;
    };
    let resident = fixture.state(|state| state.resident_count());
    let painted = fixture.state(|state| state.painted_blocks());
    assert!(resident > 0 && resident < 200, "{resident} blocks laid out");
    assert!(!painted.is_empty());
    assert!(painted.len() <= resident);
    for (_, top) in painted {
        assert!(top < VIEWPORT.height, "painted a block at {top}");
    }
    let height = fixture
        .text
        .document_height(fixture.surface, fixture.document())
        .unwrap();
    assert!(
        height > 10_000.0 * 10.0,
        "the whole document has extent: {height}"
    );
}

#[test]
fn scrolling_far_lays_out_only_where_it_lands() {
    let Some(mut fixture) = fixture(10_000, SurfaceWrap::Width) else {
        return;
    };
    let bounds = fixture.bounds();
    let center = Point::new(bounds.x() + 50.0, bounds.y() + 50.0);
    // 120 000 px down, short of the end.
    for _ in 0..30 {
        assert_eq!(
            fixture.dispatch(wheel(center, -4000.0)),
            EventResult::Consumed
        );
        fixture.frame();
    }
    let scroll = fixture.state(|state| state.scroll_offset());
    assert!(scroll.y > 100_000.0, "scrolled to {}", scroll.y);
    let resident = fixture.state(|state| state.resident_count());
    assert!(resident < 200, "{resident} blocks laid out");
    // A wheel at the very top scrolls nothing, and is left for the page.
    fixture.edit_state(|state| {
        state.scroll = Point::new(0.0, 0.0);
        state.anchor = None;
    });
    fixture.frame();
    assert_eq!(fixture.dispatch(wheel(center, 40.0)), EventResult::Ignored);
}

#[test]
fn an_insertion_above_does_not_move_what_is_on_screen() {
    let Some(mut fixture) = fixture(2_000, SurfaceWrap::Width) else {
        return;
    };
    fixture.edit_state(|state| {
        state.scroll = Point::new(0.0, 5_000.0);
        state.anchor = None;
    });
    fixture.frame();
    let watched = fixture.state(|state| {
        let (paragraph, _) = state.painted_blocks()[2];
        *state.blocks.get(&paragraph).unwrap()
    });
    let before = fixture.screen_y(watched).unwrap();

    {
        let mut model = fixture.model.borrow_mut();
        for _ in 0..25 {
            model.insert(
                10,
                "inserted above, wrapping onto more than one line of the viewport",
            );
        }
        model.remove(0);
    }
    fixture.handle.invalidate();
    fixture.frame();

    let after = fixture.screen_y(watched).expect("still on screen");
    assert!(
        (after - before).abs() < 0.5,
        "moved from {before} to {after}"
    );
    let len = fixture
        .text
        .document_len(fixture.surface, fixture.document())
        .unwrap();
    assert_eq!(len, 2_024);
}

#[test]
fn a_reveal_far_away_lands_on_the_block() {
    let Some(mut fixture) = fixture(10_000, SurfaceWrap::Width) else {
        return;
    };
    let target = fixture.model.borrow().id(9_000);
    fixture.edit_state(|state| {
        state.reveal = Some((SurfacePosition::new(target, 0), 8.0));
    });
    fixture.frame();
    let y = fixture.screen_y(target).expect("revealed block is painted");
    assert!((0.0..VIEWPORT.height).contains(&y), "at {y}");
}

#[test]
fn decorations_span_blocks_and_carets_sit_at_their_position() {
    let Some(mut fixture) = fixture(50, SurfaceWrap::Width) else {
        return;
    };
    {
        let mut model = fixture.model.borrow_mut();
        let (first, third) = (model.id(1), model.id(3));
        model.decorations = vec![
            SurfaceDecor {
                range: SurfaceRange::new(
                    SurfacePosition::new(first, 2),
                    SurfacePosition::new(third, 3),
                ),
                kind: DecorKind::Background(Color::WHITE),
                layer: DecorLayer::BelowText,
            },
            SurfaceDecor {
                range: SurfaceRange::caret(SurfacePosition::new(third, 3)),
                kind: DecorKind::Caret {
                    color: Color::BLACK,
                    width: 2.0,
                },
                layer: DecorLayer::AboveText,
            },
        ];
    }
    fixture.handle.invalidate();
    fixture.frame();
    let rects = fixture.state(|state| state.decoration_rects());
    // Part of block 1, all of block 2, part of block 3, and the caret.
    assert!(rects.len() >= 4, "{rects:?}");
    let caret = rects.last().unwrap();
    assert_eq!(caret.width, 2.0);
    let rows: Vec<f32> = rects.iter().map(|rect| rect.y).collect();
    assert!(
        rows.windows(2).take(2).all(|pair| pair[0] < pair[1]),
        "{rows:?}"
    );
}

#[test]
fn a_click_reaches_the_model_as_a_text_position() {
    let Some(mut fixture) = fixture(100, SurfaceWrap::Width) else {
        return;
    };
    let bounds = fixture.bounds();
    // Two lines down, a little way in.
    let line = fixture.state(|state| {
        let painted = state.painted_blocks();
        painted[2].1 + 2.0
    });
    let point = Point::new(bounds.x() + 20.0, bounds.y() + line);
    fixture.dispatch(RawEvent::PointerDown(pointer(point)));
    fixture.dispatch(RawEvent::PointerUp(pointer(point)));

    let model = fixture.model.borrow();
    let down = model.pointers.first().expect("the model saw the press");
    assert_eq!(down.kind, SurfacePointerKind::Down);
    assert_eq!(down.click_count, 1);
    let at = down.at.expect("resolved to a position");
    assert_eq!(at.block, model.id(2));
    assert!(
        at.offset > 0 && at.offset < "line 2".len(),
        "offset {}",
        at.offset
    );
    assert_eq!(model.pointers.last().unwrap().kind, SurfacePointerKind::Up);
}

#[test]
fn the_ime_session_follows_the_primary_caret() {
    let Some(mut fixture) = fixture(100, SurfaceWrap::Width) else {
        return;
    };
    let target = fixture.model.borrow().id(4);
    fixture.model.borrow_mut().caret = Some(SurfacePosition::new(target, 2));
    fixture.handle.invalidate();
    fixture.frame();
    let top = fixture.screen_y(target).unwrap();
    let session = fixture
        .arena
        .node_view(fixture.surface)
        .widget
        .platform_text_input_session(Bounds::from_origin_size((10.0, 20.0), VIEWPORT), None)
        .expect("a surface has a session");
    assert!(session.multiline);
    assert!(
        (session.cursor_area.y - (20.0 + top)).abs() < 1.0,
        "{session:?} vs {top}"
    );
    assert!(session.cursor_area.x > 10.0);
}

#[test]
fn unwrapped_blocks_scroll_sideways_to_a_reveal() {
    let Some(mut fixture) = fixture(10, SurfaceWrap::None) else {
        return;
    };
    let long = "x".repeat(400);
    let target = {
        let mut model = fixture.model.borrow_mut();
        let id = model.insert(3, &long);
        model.caret = Some(SurfacePosition::new(id, long.len()));
        id
    };
    fixture.handle.invalidate();
    fixture.frame();
    fixture.edit_state(|state| {
        state.reveal = Some((SurfacePosition::new(target, long.len()), 4.0));
    });
    fixture.frame();
    let scroll = fixture.state(|state| state.scroll_offset());
    assert!(scroll.x > 0.0, "did not scroll sideways: {scroll:?}");
}

fn key_a() -> RawEvent {
    RawEvent::Keyboard(xui_interface::events::RawKeyboard {
        physical_key: xui_interface::events::PhysicalKey::KeyA,
        named_key: None,
        state: xui_interface::events::KeyState::Down,
        text: xui_interface::events::KeyText::try_new("a"),
        modifiers: Modifiers::default(),
        timestamp: Instant::now(),
        is_repeat: false,
    })
}

#[test]
fn typed_text_reaches_the_focused_model() {
    let Some(mut fixture) = fixture(10, SurfaceWrap::Width) else {
        return;
    };
    let bounds = fixture.bounds();
    let point = Point::new(bounds.x() + 10.0, bounds.y() + 5.0);
    fixture.dispatch(RawEvent::PointerDown(pointer(point)));
    fixture.dispatch(RawEvent::PointerUp(pointer(point)));
    assert_eq!(fixture.arena.focused_node(), Some(fixture.surface));
    assert_eq!(fixture.dispatch(key_a()), EventResult::Consumed);
    assert_eq!(fixture.model.borrow().typed, ["a"]);
}

#[test]
fn a_wheel_the_surface_scrolls_with_leaves_its_container_alone() {
    let backend = FBackend::new();
    if backend.face_count() == 0 {
        return;
    }
    let controller = crate::scroll::ScrollController::new();
    let model = Rc::new(RefCell::new(Lines::new(1_000)));
    let mut arena = UiRuntime::new();
    let scroller = arena.create_node(Box::new(
        crate::widgets::container::container()
            .with::<crate::widgets::container::container::width>(400.0)
            .with::<crate::widgets::container::container::height>(200.0)
            .with::<crate::widgets::container::container::scroll_direction>(
                xui_interface::ScrollDirectionStyle::Vertical,
            )
            .with::<crate::widgets::container::container::scroll_controller>(controller.clone()),
    ));
    let surface = arena.create_node(Box::new(
        text_surface()
            .with::<text_surface::model>(TextSurfaceHandle::new(model))
            .with::<text_surface::size>(Size::fix(400.0, 600.0)),
    ));
    let root = arena.root();
    arena.place(root, scroller, None);
    arena.place(scroller, surface, None);
    let mut text = TextHost::new(backend);
    arena.update_tree(Size::new(800.0, 600.0), &mut text);

    let mut translator = EventTranslator::new(EventTranslatorConfig::default());
    let result = arena.dispatch_event(&text, &mut translator, wheel(Point::new(50.0, 50.0), -40.0));
    assert_eq!(result, EventResult::Consumed);
    assert_eq!(controller.offset().y, 0.0, "the container scrolled too");
}

/// Not a gate: prints what a sync costs, for comparing changes.
#[test]
fn sync_cost_per_scroll_and_per_edit() {
    let Some(mut fixture) = fixture(100_000, SurfaceWrap::Width) else {
        return;
    };
    let bounds = fixture.bounds();
    let center = Point::new(bounds.x() + 50.0, bounds.y() + 50.0);
    let frames = 60;
    let start = Instant::now();
    for _ in 0..frames {
        fixture.dispatch(wheel(center, -120.0));
        fixture.frame();
    }
    let scroll = start.elapsed() / frames;

    let start = Instant::now();
    for edit in 0..frames as usize {
        {
            let mut model = fixture.model.borrow_mut();
            let index = 50_000 + edit;
            let id = model.id(index);
            model.lines[index].1.push('!');
            model.changes.push(BlockChange::Text { id });
        }
        fixture.handle.invalidate();
        fixture.frame();
    }
    let edit = start.elapsed() / frames;
    eprintln!("text surface, 100k blocks: {scroll:?} per scrolled frame, {edit:?} per edit");
    assert!(fixture.state(|state| state.resident_count()) < 200);
}
