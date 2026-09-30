//! Tests for the host runtime, across every stage of the pipeline.

#![allow(deprecated)]

use crate::core::Point;
use crate::core::Size;
use crate::dsl::StyleProps;
use crate::event_system::callbacks::EventProps;
use crate::event_system::translator::EventTranslator;
use crate::event_system::{Flow, Handler};
use crate::focus::FocusHandle;
use crate::render::RenderNodeKind;
use crate::text::TextHost;
use crate::text::TextLayoutSlot;
use crate::text::testing::ZeroTextBackend;
use crate::ui_runtime::UiRuntime;
use crate::widgets::OverlayEntryId;
use crate::widgets::OverlayEntryOptions;
use crate::widgets::WidgetType;
use crate::widgets::Widgets;
use crate::widgets::canvas_text_slot;
use crate::widgets::{
    CanvasController, TextWidget, WidgetI, canvas, container, text_input, z_stack,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};
use xui_animation::{Easing, Transition};
use xui_interface::Bounds;
use xui_interface::ComputedStyle;
use xui_interface::CursorIcon;
use xui_interface::EventResult;
use xui_interface::NodeId;
use xui_interface::TextLayoutConstraints;
use xui_interface::TextLayoutInput;
use xui_interface::Theme;
use xui_interface::WidgetUpdateFlags;
use xui_interface::core::Sizing;
use xui_interface::events::RawEvent;
use xui_interface::events::semantic::ClickEvent;
use xui_interface::events::{
    EventPhase, Modifiers, PointerButton, PointerButtons, PointerKind, RawPointerButton,
    RawPointerMove, XuiPointerId,
};
use xui_interface::style::FlexDirectionStyle;
use xui_interface::{
    Affine, CanvasTextId, Color, ComputedColorStyle, EdgeInsets, FontDatabase, PathBuilder,
    PathFill, Style, TextProps, VectorSceneBuilder, WidgetState,
};

fn create_host(arena: &mut UiRuntime, widget: WidgetI) -> NodeId {
    let parent = arena.root();
    let key = widget.key();
    let props_hash = widget.props_hash();
    let interaction = widget.take_host_interaction();
    let id = arena.create_node(key, props_hash, widget, interaction);
    arena.place(parent, id, None);
    id
}

fn canvas_generation(arena: &UiRuntime, id: NodeId) -> u64 {
    arena.hosts[id].widget.with_widgets(|widget| match widget {
        Widgets::Canvas(canvas) => canvas.compiled_generation(),
        _ => panic!("not a canvas node"),
    })
}

fn canvas_compiled_size(arena: &UiRuntime, id: NodeId) -> Size<f32> {
    arena.hosts[id].widget.with_widgets(|widget| match widget {
        Widgets::Canvas(canvas) => canvas.compiled_size(),
        _ => panic!("not a canvas node"),
    })
}

#[test]
fn a_controller_edit_repaints_its_canvas_with_no_component_rebuild() {
    let mut arena = UiRuntime::new();
    let mut measurer = TextHost::new(ZeroTextBackend);
    let controller = CanvasController::new();
    let id = create_host(
        &mut arena,
        WidgetI::new(
            canvas()
                .controller(controller.clone())
                .style(Style::new().width(80.0).height(40.0)),
        ),
    );

    arena.update_tree(Size::new(400.0, 200.0), &mut measurer);
    let before = canvas_generation(&arena, id);
    assert!(before > 0, "a mounted canvas is compiled before it paints");
    assert!(arena.canvas_invalidations.is_empty());

    let mut path = PathBuilder::new();
    path.move_to(Point::new(0.0, 0.0))
        .line_to(Point::new(10.0, 10.0));
    let mut scene = VectorSceneBuilder::new();
    scene.fill_path(path.build(), Affine::IDENTITY, PathFill::new(Color::BLACK));
    controller.set_scene(scene.build());

    assert!(
        !arena.canvas_invalidations.is_empty(),
        "the edit must reach the host directly"
    );
    assert!(
        arena.is_dirty(),
        "and it must be enough on its own to schedule a frame"
    );

    let visits_before = arena.stats.update_visits;
    arena.update_tree(Size::new(400.0, 200.0), &mut measurer);
    assert!(canvas_generation(&arena, id) > before);
    assert!(arena.canvas_invalidations.is_empty());
    let _ = visits_before;
}

#[test]
fn a_painter_is_re_run_against_the_size_layout_committed() {
    let sizes = Rc::new(RefCell::new(Vec::new()));
    let recorder = sizes.clone();
    let controller = CanvasController::with_painter(move |painter| {
        recorder.borrow_mut().push(painter.size());
        let width = painter.width();
        painter.rect(
            Bounds::from_origin_size((0.0, 0.0), (width, 4.0)),
            Color::BLACK,
        );
    });

    let mut arena = UiRuntime::new();
    let mut measurer = TextHost::new(ZeroTextBackend);
    let id = create_host(
        &mut arena,
        WidgetI::new(
            canvas()
                .controller(controller.clone())
                .style(Style::new().size(Size::fill())),
        ),
    );

    arena.update_tree(Size::new(400.0, 200.0), &mut measurer);
    assert_eq!(canvas_compiled_size(&arena, id), Size::new(400.0, 200.0));
    assert_eq!(controller.size(), Size::new(400.0, 200.0));

    arena.mark_subtree_layout_dirty(arena.root());
    arena.update_tree(Size::new(640.0, 200.0), &mut measurer);
    assert_eq!(canvas_compiled_size(&arena, id), Size::new(640.0, 200.0));
    assert_eq!(
        sizes.borrow().last().copied(),
        Some(Size::new(640.0, 200.0)),
        "the painter has to see the measured size, not the one it was authored for"
    );
}

#[test]
fn a_painter_that_draws_nothing_new_is_not_re_run_when_only_the_viewport_moves() {
    let runs = Rc::new(Cell::new(0));
    let counter = runs.clone();
    let controller = CanvasController::with_painter(move |_| {
        counter.set(counter.get() + 1);
    });

    let mut arena = UiRuntime::new();
    let mut measurer = TextHost::new(ZeroTextBackend);
    create_host(
        &mut arena,
        WidgetI::new(
            canvas()
                .controller(controller)
                .style(Style::new().width(80.0).height(40.0)),
        ),
    );

    arena.update_tree(Size::new(400.0, 200.0), &mut measurer);
    let after_mount = runs.get();

    arena.mark_subtree_layout_dirty(arena.root());
    arena.update_tree(Size::new(640.0, 200.0), &mut measurer);
    assert_eq!(
        runs.get(),
        after_mount,
        "a fixed-size canvas keeps its drawing when the window resizes around it"
    );
}

#[test]
fn text_a_painter_produces_is_shaped_before_the_canvas_paints() {
    let controller = CanvasController::with_painter(|painter| {
        let width = painter.width();
        painter.text(
            Bounds::from_origin_size((0.0, 0.0), (width, 20.0)),
            TextProps::new("measured after layout"),
        );
    });

    let mut arena = UiRuntime::new();
    let mut measurer = TextHost::new(ZeroTextBackend);
    let id = create_host(
        &mut arena,
        WidgetI::new(
            canvas()
                .controller(controller)
                .style(Style::new().size(Size::fill())),
        ),
    );

    arena.update_tree(Size::new(400.0, 200.0), &mut measurer);
    assert!(
        measurer
            .active_slot(id, canvas_text_slot(CanvasTextId::new(1)))
            .is_some(),
        "a painter's text box has to reach the shaper in the same frame it is drawn"
    );
}

#[test]
fn canvas_text_layout_is_measured_and_activated_in_the_same_compile() {
    let observed = Rc::new(RefCell::new(None));
    let sink = observed.clone();
    let controller = CanvasController::with_painter(move |painter| {
        let layout = painter.layout_text_keyed(
            9,
            TextProps::new("measure then draw"),
            TextLayoutConstraints::max_width(painter.width()),
        );
        *sink.borrow_mut() = Some(layout.metrics());
        painter.draw_text(Point::new(4.0, 6.0), layout);
    });

    let mut arena = UiRuntime::new();
    let mut measurer = TextHost::new(ZeroTextBackend);
    let id = create_host(
        &mut arena,
        WidgetI::new(
            canvas()
                .controller(controller)
                .style(Style::new().width(120.0).height(40.0)),
        ),
    );

    arena.update_tree(Size::new(400.0, 200.0), &mut measurer);

    let metrics = observed
        .borrow()
        .expect("the painter should receive shaped text metrics");
    assert_eq!(metrics.size, Size::<f32>::ZERO);
    assert_eq!(metrics.first_baseline, None);
    assert_eq!(metrics.line_count, 0);
    assert!(
        measurer
            .active_slot(id, canvas_text_slot(CanvasTextId::new((1 << 31) | 9)))
            .is_some(),
        "the measured variant must also become the active paint layout"
    );
}

#[test]
fn removing_a_canvas_stops_its_controller_from_marking_the_dead_node() {
    let mut arena = UiRuntime::new();
    let mut measurer = TextHost::new(ZeroTextBackend);
    let controller = CanvasController::new();
    let id = create_host(
        &mut arena,
        WidgetI::new(
            canvas()
                .controller(controller.clone())
                .style(Style::new().width(80.0).height(40.0)),
        ),
    );
    arena.update_tree(Size::new(400.0, 200.0), &mut measurer);

    arena.remove_subtree(id);
    controller.invalidate();
    assert!(
        arena.canvas_invalidations.is_empty(),
        "an unmounted canvas must not keep a queue entry alive"
    );
}

#[test]
fn runtime_keeps_one_root_overlayer_last_without_blocking_content_hits() {
    let mut arena = UiRuntime::new();
    let content = create_host(
        &mut arena,
        WidgetI::new(container().style(Style::new().size(Size::fill()))),
    );
    let overlayer = arena.root_overlayer();
    let mut measurer = TextHost::new(ZeroTextBackend);

    arena.update_tree(Size::new(400.0, 200.0), &mut measurer);

    assert_eq!(
        arena
            .children(arena.root())
            .filter(|id| arena.node(*id).unwrap().node_type == WidgetType::RootOverlayer)
            .count(),
        1
    );
    assert_eq!(arena.children(arena.root()).last(), Some(overlayer));
    assert_eq!(
        arena.node(overlayer).unwrap().layout.size(),
        Size::new(400.0, 200.0)
    );
    assert_eq!(arena.hit_test(Point::new(20.0, 20.0)), Some(content));

    arena.remove_subtree(content);
    assert!(arena.contains(overlayer));
    assert_eq!(
        arena.children(arena.root()).collect::<Vec<_>>(),
        vec![overlayer]
    );
}

#[test]
fn hit_test_tracks_scrolled_content_without_moving_the_scroll_viewport() {
    let mut arena = UiRuntime::new();
    let scroll = create_host(
        &mut arena,
        WidgetI::new(container().style(Style::new().width(100.0).height(100.0).scroll_vertical())),
    );
    let content = create_host(
        &mut arena,
        WidgetI::new(container().style(Style::new().width(20.0).height(200.0))),
    );
    let target = create_host(
        &mut arena,
        WidgetI::new(
            container().style(
                Style::new()
                    .absolute()
                    .inset(xui_interface::EdgeInsets::new(0.0, 0.0, 80.0, 0.0))
                    .width(20.0)
                    .height(20.0),
            ),
        ),
    );
    arena.place(scroll, content, None);
    arena.place(content, target, None);

    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(200.0, 200.0), &mut measurer);
    assert!(arena.set_scroll_offset(scroll, Point::new(0.0, 60.0)));

    assert_eq!(arena.visual_layout(target).y(), 20.0);
    assert_eq!(arena.node(target).unwrap().world_origin.y, 20.0);
    // Event handlers get the same scrolled position, e.g. to anchor a menu.
    assert_eq!(arena.node_view(target).world_origin.y, 20.0);
    assert_eq!(arena.hit_test(Point::new(10.0, 25.0)), Some(target));
    assert_ne!(arena.hit_test(Point::new(10.0, 85.0)), Some(target));
    assert_eq!(arena.hit_test(Point::new(50.0, 90.0)), Some(scroll));
}

#[test]
fn hit_test_accumulates_nested_scroll_offsets() {
    let mut arena = UiRuntime::new();
    let outer = create_host(
        &mut arena,
        WidgetI::new(container().style(Style::new().width(120.0).height(100.0).scroll_vertical())),
    );
    let inner = create_host(
        &mut arena,
        WidgetI::new(
            container().style(
                Style::new()
                    .absolute()
                    .inset(xui_interface::EdgeInsets::new(0.0, 0.0, 40.0, 0.0))
                    .width(100.0)
                    .height(80.0)
                    .scroll_vertical(),
            ),
        ),
    );
    let target = create_host(
        &mut arena,
        WidgetI::new(
            container().style(
                Style::new()
                    .absolute()
                    .inset(xui_interface::EdgeInsets::new(0.0, 0.0, 50.0, 0.0))
                    .width(20.0)
                    .height(20.0),
            ),
        ),
    );
    arena.place(outer, inner, None);
    arena.place(inner, target, None);

    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(200.0, 200.0), &mut measurer);
    assert!(arena.set_scroll_offset(outer, Point::new(0.0, 20.0)));
    assert!(arena.set_scroll_offset(inner, Point::new(0.0, 30.0)));

    assert_eq!(arena.visual_layout(target).y(), 40.0);
    assert_eq!(arena.hit_test(Point::new(10.0, 45.0)), Some(target));
    assert_ne!(arena.hit_test(Point::new(10.0, 95.0)), Some(target));
}

#[test]
fn hit_test_allows_unclipped_overflow_and_respects_rounded_clips() {
    let mut arena = UiRuntime::new();
    let overflow_parent = create_host(
        &mut arena,
        WidgetI::new(container().style(Style::new().width(40.0).height(40.0))),
    );
    let overflow_child = create_host(
        &mut arena,
        WidgetI::new(
            container().style(
                Style::new()
                    .absolute()
                    .inset(xui_interface::EdgeInsets::new(50.0, 0.0, 0.0, 0.0))
                    .width(20.0)
                    .height(20.0),
            ),
        ),
    );
    arena.place(overflow_parent, overflow_child, None);
    let corner_child = create_host(
        &mut arena,
        WidgetI::new(container().style(Style::new().width(40.0).height(40.0))),
    );
    arena.place(overflow_parent, corner_child, None);

    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
    assert_eq!(arena.hit_test(Point::new(55.0, 10.0)), Some(overflow_child));

    update_host(
        &mut arena,
        overflow_parent,
        WidgetI::new(
            container().style(
                Style::new()
                    .width(40.0)
                    .height(40.0)
                    .clip(true)
                    .border_radius(20.0),
            ),
        ),
    );
    arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
    assert_eq!(arena.hit_test(Point::new(1.0, 1.0)), Some(arena.root()));
    assert_eq!(arena.hit_test(Point::new(20.0, 20.0)), Some(corner_child));
    assert_ne!(arena.hit_test(Point::new(55.0, 10.0)), Some(overflow_child));
}

#[test]
fn root_overlayer_honors_hit_test_and_modal_stacking() {
    let mut arena = UiRuntime::new();
    let content = create_host(
        &mut arena,
        WidgetI::new(container().style(Style::new().size(Size::fill()))),
    );
    let pass_through = create_host(
        &mut arena,
        WidgetI::new(
            container().style(
                Style::new()
                    .absolute()
                    .inset(xui_interface::EdgeInsets::zero())
                    .width(40.0)
                    .height(40.0),
            ),
        ),
    );
    let pass_through_entry = arena
        .mount_overlay_entry(
            pass_through,
            None,
            OverlayEntryOptions {
                hit_test: false,
                ..Default::default()
            },
        )
        .unwrap();

    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
    assert_eq!(arena.hit_test(Point::new(10.0, 10.0)), Some(content));

    arena
        .update_overlay_entry(pass_through_entry, None, OverlayEntryOptions::default())
        .unwrap();
    assert_eq!(arena.hit_test(Point::new(10.0, 10.0)), Some(pass_through));

    arena
        .update_overlay_entry(
            pass_through_entry,
            None,
            OverlayEntryOptions {
                modal: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(arena.hit_test(Point::new(80.0, 80.0)), None);
}

#[test]
fn hit_test_uses_reverse_paint_order_for_overlapping_layers() {
    let mut arena = UiRuntime::new();
    let stack = create_host(
        &mut arena,
        WidgetI::new(z_stack().style(Style::new().width(50.0).height(50.0))),
    );
    let back = create_host(
        &mut arena,
        WidgetI::new(container().style(Style::new().width(50.0).height(50.0))),
    );
    let front = create_host(
        &mut arena,
        WidgetI::new(container().style(Style::new().width(50.0).height(50.0))),
    );
    arena.place(stack, back, None);
    arena.place(stack, front, None);

    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
    assert_eq!(arena.hit_test(Point::new(10.0, 10.0)), Some(front));
}

fn update_host(arena: &mut UiRuntime, id: NodeId, widget: WidgetI) {
    let key = widget.key();
    let props_hash = widget.props_hash();
    let interaction = widget.take_host_interaction();
    arena.update_node(id, key, props_hash, widget, interaction);
}

// ---------------------------------------------------------------------
// Event system: handler sharing, Flow, and the widget stage
// ---------------------------------------------------------------------

fn pointer_at(position: Point) -> RawPointerButton {
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

/// Clicks the centre of `node` by driving the real raw-event pipeline, so
/// the semantic events under test are the ones the translator actually
/// produces.
fn click(arena: &mut UiRuntime, measurer: &TextHost<ZeroTextBackend>, node: NodeId) {
    let bounds = arena.node(node).unwrap().layout;
    let point = Point::new(
        (bounds.min.x + bounds.max.x) * 0.5,
        (bounds.min.y + bounds.max.y) * 0.5,
    );
    let mut translator =
        EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());
    arena.dispatch_event(measurer, &mut translator, pointer_move(point));
    arena.dispatch_event(
        measurer,
        &mut translator,
        RawEvent::PointerDown(pointer_at(point)),
    );
    arena.dispatch_event(
        measurer,
        &mut translator,
        RawEvent::PointerUp(pointer_at(point)),
    );
}

#[test]
fn pressing_a_non_focusable_area_clears_focus() {
    let mut arena = UiRuntime::new();
    let field = create_host(
        &mut arena,
        WidgetI::new(
            container()
                .tab_index(0)
                .style(Style::new().size(Size::fix(40.0, 40.0))),
        ),
    );
    let blank = create_host(
        &mut arena,
        WidgetI::new(container().style(Style::new().size(Size::fix(40.0, 40.0)))),
    );
    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(200.0, 200.0), &mut measurer);

    click(&mut arena, &measurer, field);
    assert_eq!(arena.focused_node(), Some(field));

    click(&mut arena, &measurer, blank);
    assert_eq!(arena.focused_node(), None);
    assert!(
        !arena
            .node(field)
            .unwrap()
            .state
            .contains(WidgetState::FOCUSED)
    );
}

#[test]
fn only_keyboard_focus_is_focus_visible() {
    use xui_interface::events::{KeyState, NamedKey, PhysicalKey, RawKeyboard};

    let mut arena = UiRuntime::new();
    let first = create_host(
        &mut arena,
        WidgetI::new(
            container()
                .tab_index(0)
                .style(Style::new().size(Size::fix(40.0, 40.0))),
        ),
    );
    let second = create_host(
        &mut arena,
        WidgetI::new(
            container()
                .tab_index(0)
                .style(Style::new().size(Size::fix(40.0, 40.0))),
        ),
    );
    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(200.0, 200.0), &mut measurer);
    let state = |arena: &UiRuntime, node| arena.node(node).unwrap().state;

    click(&mut arena, &measurer, first);
    assert!(state(&arena, first).contains(WidgetState::FOCUSED));
    assert!(!state(&arena, first).contains(WidgetState::FOCUS_VISIBLE));

    let mut translator =
        EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());
    arena.dispatch_event(
        &measurer,
        &mut translator,
        RawEvent::Keyboard(RawKeyboard {
            physical_key: PhysicalKey::Unidentified,
            named_key: Some(NamedKey::Tab),
            state: KeyState::Down,
            text: None,
            modifiers: Modifiers::default(),
            timestamp: Instant::now(),
            is_repeat: false,
        }),
    );
    assert_eq!(arena.focused_node(), Some(second));
    assert!(state(&arena, second).contains(WidgetState::FOCUS_VISIBLE));
    assert!(!state(&arena, first).contains(WidgetState::FOCUS_VISIBLE));
}

/// One handler, two widgets. Impossible with the previous
/// `Box<dyn FnMut>`, which could be neither cloned nor shared, and the
/// reason a component could not accept an event handler as a prop.
#[test]
fn one_handler_can_be_attached_to_two_widgets() {
    let calls = Rc::new(Cell::new(0u32));
    let handler = {
        let calls = Rc::clone(&calls);
        Handler::<ClickEvent>::new(move |_, _| {
            calls.set(calls.get() + 1);
        })
    };

    let mut arena = UiRuntime::new();
    let left = create_host(
        &mut arena,
        WidgetI::new(
            container()
                .style(Style::new().size(Size::fix(40.0, 40.0)))
                .on_click(handler.clone().into_fn()),
        ),
    );
    let right = create_host(
        &mut arena,
        WidgetI::new(
            container()
                .style(Style::new().size(Size::fix(40.0, 40.0)))
                .on_click(handler.clone().into_fn()),
        ),
    );

    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(200.0, 200.0), &mut measurer);

    click(&mut arena, &measurer, left);
    click(&mut arena, &measurer, right);

    assert_eq!(calls.get(), 2, "the shared handler did not run on both");
    assert!(handler.ptr_eq(&handler.clone()), "cloning changed identity");
}

/// Hovering a leaf marks every ancestor on the way to it, each exactly once.
///
/// The translator diffs the old and new root-to-target paths, so the whole
/// entered segment is reported — not just the leaf. This is what makes
/// `style!(.. if hovered ..)` work on a container whose child is under the
/// pointer.
#[test]
fn entering_a_leaf_reports_every_ancestor_that_gained_hover_once_each() {
    let counts: Rc<RefCell<Vec<(&'static str, bool)>>> = Rc::new(RefCell::new(Vec::new()));

    let mut arena = UiRuntime::new();

    let outer_log = Rc::clone(&counts);
    let outer = create_host(
        &mut arena,
        WidgetI::new(
            container()
                .style(Style::new().size(Size::fix(100.0, 100.0)))
                .on_hovered(move |event, _| {
                    outer_log.borrow_mut().push(("outer", event.hovered));
                }),
        ),
    );

    let mut nest = |arena: &mut UiRuntime, parent: NodeId, name: &'static str, size: f32| {
        let log = Rc::clone(&counts);
        let widget = WidgetI::new(
            container()
                .style(Style::new().size(Size::fix(size, size)))
                .on_hovered(move |event, _| {
                    log.borrow_mut().push((name, event.hovered));
                }),
        );
        let key = widget.key();
        let props_hash = widget.props_hash();
        let interaction = widget.take_host_interaction();
        let id = arena.create_node(key, props_hash, widget, interaction);
        arena.place(parent, id, None);
        id
    };

    let middle = nest(&mut arena, outer, "middle", 80.0);
    let leaf = nest(&mut arena, middle, "leaf", 60.0);

    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(200.0, 200.0), &mut measurer);

    let bounds = arena.node(leaf).unwrap().layout;
    let inside = Point::new(
        (bounds.min.x + bounds.max.x) * 0.5,
        (bounds.min.y + bounds.max.y) * 0.5,
    );
    let mut translator =
        EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());
    arena.dispatch_event(&measurer, &mut translator, pointer_move(inside));

    let seen = counts.borrow().clone();
    for name in ["outer", "middle", "leaf"] {
        let entered = seen
            .iter()
            .filter(|(node, hovered)| *node == name && *hovered)
            .count();
        assert_eq!(
            entered, 1,
            "`{name}` should have been reported hovered exactly once, got {entered} in {seen:?}"
        );
    }
    assert!(
        !seen.iter().any(|(_, hovered)| !*hovered),
        "nothing left the hover path yet, but a leave was reported: {seen:?}"
    );
}

#[test]
fn pointer_move_is_typed_bubbles_and_maps_current_local_per_handler() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut arena = UiRuntime::new();

    let capture_seen = Rc::clone(&seen);
    let bubble_seen = Rc::clone(&seen);
    let parent = create_host(
        &mut arena,
        WidgetI::new(
            container()
                .style(
                    Style::new()
                        .size(Size::fix(100.0, 100.0))
                        .padding(EdgeInsets::all(20.0)),
                )
                .on_pointer_move_capture(move |event, _| {
                    capture_seen.borrow_mut().push((
                        event.meta.phase,
                        event.pointer.coords.target_local,
                        event.pointer.coords.current_local,
                    ));
                })
                .on_pointer_move(move |event, _| {
                    bubble_seen.borrow_mut().push((
                        event.meta.phase,
                        event.pointer.coords.target_local,
                        event.pointer.coords.current_local,
                    ));
                }),
        ),
    );

    let target_seen = Rc::clone(&seen);
    let child = {
        let widget = WidgetI::new(
            container()
                .style(Style::new().size(Size::fix(40.0, 40.0)))
                .on_pointer_move(move |event, _| {
                    target_seen.borrow_mut().push((
                        event.meta.phase,
                        event.pointer.coords.target_local,
                        event.pointer.coords.current_local,
                    ));
                }),
        );
        let key = widget.key();
        let props_hash = widget.props_hash();
        let interaction = widget.take_host_interaction();
        let id = arena.create_node(key, props_hash, widget, interaction);
        arena.place(parent, id, None);
        id
    };

    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(200.0, 200.0), &mut measurer);
    let viewport = arena.node(child).unwrap().layout.min + Point::new(5.0, 7.0);
    let expected_target_local = arena.to_local(child, viewport);
    let expected_parent_local = arena.to_local(parent, viewport);

    let mut translator =
        EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());
    arena.dispatch_event(&measurer, &mut translator, pointer_move(viewport));

    assert_eq!(
        *seen.borrow(),
        vec![
            (
                EventPhase::Capture,
                expected_target_local,
                expected_parent_local,
            ),
            (
                EventPhase::Target,
                expected_target_local,
                expected_target_local,
            ),
            (
                EventPhase::Bubble,
                expected_target_local,
                expected_parent_local,
            ),
        ]
    );
}

#[test]
fn pointer_enter_and_leave_are_direct_and_report_the_other_leaf() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut arena = UiRuntime::new();

    let parent_enter = Rc::clone(&seen);
    let parent_leave = Rc::clone(&seen);
    let parent = create_host(
        &mut arena,
        WidgetI::new(
            container()
                .style(Style::new().size(Size::fix(100.0, 100.0)))
                .on_pointer_enter(move |event, _| {
                    parent_enter
                        .borrow_mut()
                        .push(("parent", "enter", event.related_target));
                })
                .on_pointer_leave(move |event, _| {
                    parent_leave
                        .borrow_mut()
                        .push(("parent", "leave", event.related_target));
                }),
        ),
    );

    let add_child = |arena: &mut UiRuntime, name: &'static str| {
        let enter_seen = Rc::clone(&seen);
        let leave_seen = Rc::clone(&seen);
        let widget = WidgetI::new(
            container()
                .style(Style::new().size(Size::fix(40.0, 40.0)))
                .on_pointer_enter(move |event, _| {
                    enter_seen
                        .borrow_mut()
                        .push((name, "enter", event.related_target));
                })
                .on_pointer_leave(move |event, _| {
                    leave_seen
                        .borrow_mut()
                        .push((name, "leave", event.related_target));
                }),
        );
        let key = widget.key();
        let props_hash = widget.props_hash();
        let interaction = widget.take_host_interaction();
        let id = arena.create_node(key, props_hash, widget, interaction);
        arena.place(parent, id, None);
        id
    };

    let left = add_child(&mut arena, "left");
    let right = add_child(&mut arena, "right");
    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(200.0, 200.0), &mut measurer);

    let centre = |arena: &UiRuntime, node: NodeId| {
        let bounds = arena.node(node).unwrap().layout;
        Point::new(
            (bounds.min.x + bounds.max.x) * 0.5,
            (bounds.min.y + bounds.max.y) * 0.5,
        )
    };
    let mut translator =
        EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());
    let on_left = centre(&arena, left);
    arena.dispatch_event(&measurer, &mut translator, pointer_move(on_left));
    seen.borrow_mut().clear();

    let on_right = centre(&arena, right);
    arena.dispatch_event(&measurer, &mut translator, pointer_move(on_right));

    assert_eq!(
        *seen.borrow(),
        vec![
            ("left", "leave", Some(right)),
            ("right", "enter", Some(left)),
        ],
        "the unchanged parent must not receive a bubbled boundary event"
    );
}

/// `Hovered` is `Direct`, so an ancestor is reached because it is genuinely
/// on the hover path — never a second time by a child's event bubbling up.
/// Making it bubble would double-report every ancestor.
#[test]
fn an_ancestor_is_not_reported_twice_when_a_child_is_hovered() {
    let outer_events = Rc::new(Cell::new(0u32));
    let mut arena = UiRuntime::new();

    let counter = Rc::clone(&outer_events);
    let outer = create_host(
        &mut arena,
        WidgetI::new(
            container()
                .style(Style::new().size(Size::fix(100.0, 100.0)))
                .on_hovered(move |_, _| counter.set(counter.get() + 1)),
        ),
    );
    let child = {
        let widget = WidgetI::new(container().style(Style::new().size(Size::fix(60.0, 60.0))));
        let key = widget.key();
        let props_hash = widget.props_hash();
        let interaction = widget.take_host_interaction();
        let id = arena.create_node(key, props_hash, widget, interaction);
        arena.place(outer, id, None);
        id
    };

    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(200.0, 200.0), &mut measurer);

    let bounds = arena.node(child).unwrap().layout;
    let inside = Point::new(
        (bounds.min.x + bounds.max.x) * 0.5,
        (bounds.min.y + bounds.max.y) * 0.5,
    );
    let mut translator =
        EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());
    arena.dispatch_event(&measurer, &mut translator, pointer_move(inside));

    assert_eq!(
        outer_events.get(),
        1,
        "the ancestor saw its own event plus a bubbled copy of the child's"
    );
}

/// `cursor` lives in `ComputedStyle` but must never reach `StyleDiffFlags`.
///
/// It has no scene output, so treating it like other style properties would
/// make moving the pointer across a button dirty layout, paint, or text.
/// This is the guard against someone completing the `diff` match later.
#[test]
fn changing_the_cursor_dirties_nothing() {
    let theme = Theme::default();
    let base = ComputedStyle::initial(&theme);
    let mut with_cursor = base.clone();
    with_cursor.cursor = Some(CursorIcon::Pointer);

    assert_ne!(
        base.cursor, with_cursor.cursor,
        "the fixture is not testing anything"
    );
    assert!(
        base.diff(&with_cursor).is_empty(),
        "a cursor change produced invalidation flags"
    );
}

/// Not inherited in the computed style: resolution walks up instead, so a
/// child that specifies nothing shows its ancestor's cursor.
#[test]
fn an_unspecified_cursor_resolves_from_the_nearest_ancestor() {
    let mut arena = UiRuntime::new();
    let outer = create_host(
        &mut arena,
        WidgetI::new(
            container()
                .style(Style::new().size(Size::fix(100.0, 100.0)))
                .cursor(CursorIcon::Pointer),
        ),
    );
    let inner = {
        let widget = WidgetI::new(container().style(Style::new().size(Size::fix(50.0, 50.0))));
        let key = widget.key();
        let props_hash = widget.props_hash();
        let interaction = widget.take_host_interaction();
        let id = arena.create_node(key, props_hash, widget, interaction);
        arena.place(outer, id, None);
        id
    };

    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(200.0, 200.0), &mut measurer);

    assert_eq!(
        arena.style_system.styles(inner).1.cursor,
        None,
        "the child should not have inherited a resolved cursor"
    );

    let bounds = arena.node(inner).unwrap().layout;
    let inside = Point::new(
        (bounds.min.x + bounds.max.x) * 0.5,
        (bounds.min.y + bounds.max.y) * 0.5,
    );
    let mut translator =
        EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());
    arena.dispatch_event(&measurer, &mut translator, pointer_move(inside));

    assert_eq!(arena.hovered_node(), Some(inner));
    assert_eq!(arena.resolved_cursor(), CursorIcon::Pointer);
}

type ScrollLog = Rc<
    RefCell<
        Vec<(
            xui_interface::events::ScrollSource,
            xui_interface::events::ScrollPhase,
            f32,
        )>,
    >,
>;

/// A 100x100 vertical scroller over 400px of content, whose default 8px
/// scrollbar has a 25px thumb and 75px of travel for 300px of offset.
fn scrollbar_fixture(
    arena: &mut UiRuntime,
    controller: &crate::scroll::ScrollController,
    log: &ScrollLog,
    presses: &Rc<Cell<usize>>,
) -> NodeId {
    let log = Rc::clone(log);
    let presses = Rc::clone(presses);
    let scroll = create_host(
        arena,
        WidgetI::new(
            container()
                .style(Style::new().width(100.0).height(100.0).scroll_vertical())
                .scroll_controller(controller.clone())
                .on_scroll(move |event, _| {
                    // A wheel also reports, and bubbles, a scroll for each
                    // non-scrolling node on its chain; those carry no offset.
                    if let Some(after) = event.offset_after {
                        log.borrow_mut().push((event.source, event.phase, after.y));
                    }
                }),
        ),
    );
    let content = create_host(
        arena,
        WidgetI::new(
            container()
                .style(Style::new().width(100.0).height(400.0))
                .on_press_start(move |_, _| presses.set(presses.get() + 1)),
        ),
    );
    arena.place(scroll, content, None);
    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(200.0, 200.0), &mut measurer);
    scroll
}

#[test]
fn a_scroll_controller_reads_metrics_and_applies_requests_in_order() {
    let mut arena = UiRuntime::new();
    let controller = crate::scroll::ScrollController::new();
    let scroll = scrollbar_fixture(
        &mut arena,
        &controller,
        &ScrollLog::default(),
        &Rc::default(),
    );

    assert_eq!(controller.node_id(), Some(scroll));
    let metrics = controller.metrics();
    assert_eq!(metrics.viewport_size, Size::new(100.0, 100.0));
    assert_eq!(metrics.max_offset, Point::new(0.0, 300.0));

    assert!(controller.scroll_to_end());
    assert!(controller.scroll_by((0.0, -20.0)));
    assert!(arena.is_dirty(), "a pending request must schedule a frame");

    let applied = arena.apply_scroll_requests();
    assert_eq!(applied.len(), 2);
    assert_eq!(controller.offset(), Point::new(0.0, 280.0));
    assert_eq!(arena.node(scroll).unwrap().scroll_offset.y, 280.0);
    assert!(!arena.has_pending_scroll_requests());

    controller.scroll_to_y(-50.0);
    arena.apply_scroll_requests();
    assert_eq!(controller.offset().y, 0.0, "requests clamp to the range");
}

#[test]
fn a_removed_scroller_unbinds_its_controller() {
    let mut arena = UiRuntime::new();
    let controller = crate::scroll::ScrollController::new();
    let scroll = scrollbar_fixture(
        &mut arena,
        &controller,
        &ScrollLog::default(),
        &Rc::default(),
    );

    arena.remove_subtree(scroll);
    assert!(!controller.is_bound());
    assert!(!controller.scroll_to_end());
}

#[test]
fn a_wheel_scroll_is_visible_through_the_controller_at_once() {
    let mut arena = UiRuntime::new();
    let controller = crate::scroll::ScrollController::new();
    scrollbar_fixture(
        &mut arena,
        &controller,
        &ScrollLog::default(),
        &Rc::default(),
    );

    let measurer = TextHost::new(ZeroTextBackend);
    let mut translator = EventTranslator::default();
    arena.dispatch_event(
        &measurer,
        &mut translator,
        RawEvent::Wheel(xui_interface::events::RawWheel {
            position: Point::new(50.0, 50.0),
            delta: xui_interface::events::ScrollDelta::Pixels(xui_interface::Translation::new(
                0.0, -40.0,
            )),
            device_id: None,
            pointer_id: None,
            modifiers: Modifiers::default(),
            timestamp: Instant::now(),
            is_inertial: false,
        }),
    );

    assert_eq!(controller.offset().y, 40.0);
}

#[test]
fn dragging_the_scrollbar_thumb_scrolls_without_pressing_content() {
    use xui_interface::events::{ScrollPhase, ScrollSource};

    let mut arena = UiRuntime::new();
    let log = ScrollLog::default();
    let presses = Rc::new(Cell::new(0));
    let scroll = scrollbar_fixture(
        &mut arena,
        &crate::scroll::ScrollController::new(),
        &log,
        &presses,
    );
    let measurer = TextHost::new(ZeroTextBackend);
    let mut translator = EventTranslator::default();
    let offset = |arena: &UiRuntime| arena.node(scroll).unwrap().scroll_offset.y;

    arena.dispatch_event(
        &measurer,
        &mut translator,
        RawEvent::PointerDown(pointer_at(Point::new(96.0, 10.0))),
    );
    assert_eq!(arena.pointer_capture_node(), Some(scroll));

    // 25px of thumb movement over 75px of travel is a third of 300px.
    arena.dispatch_event(
        &measurer,
        &mut translator,
        pointer_move(Point::new(96.0, 35.0)),
    );
    assert_eq!(offset(&arena), 100.0);

    // Leaving the bar keeps dragging; running past the end clamps.
    arena.dispatch_event(
        &measurer,
        &mut translator,
        pointer_move(Point::new(10.0, 500.0)),
    );
    assert_eq!(offset(&arena), 300.0);

    arena.dispatch_event(
        &measurer,
        &mut translator,
        RawEvent::PointerUp(pointer_at(Point::new(10.0, 500.0))),
    );
    assert_eq!(arena.pointer_capture_node(), None);
    arena.dispatch_event(
        &measurer,
        &mut translator,
        pointer_move(Point::new(96.0, 10.0)),
    );
    assert_eq!(offset(&arena), 300.0, "the drag ended with the release");

    assert_eq!(
        *log.borrow(),
        vec![
            (ScrollSource::Scrollbar, ScrollPhase::Start, 0.0),
            (ScrollSource::Scrollbar, ScrollPhase::Move, 100.0),
            (ScrollSource::Scrollbar, ScrollPhase::Move, 300.0),
            (ScrollSource::Scrollbar, ScrollPhase::End, 300.0),
        ],
        "a drag reports Start, its moves, then End on release"
    );
    assert_eq!(presses.get(), 0, "content under the bar was pressed");
}

#[test]
fn a_press_on_the_scrollbar_track_pages_toward_the_pointer() {
    use xui_interface::events::{ScrollPhase, ScrollSource};

    let mut arena = UiRuntime::new();
    let log = ScrollLog::default();
    let presses = Rc::new(Cell::new(0));
    let scroll = scrollbar_fixture(
        &mut arena,
        &crate::scroll::ScrollController::new(),
        &log,
        &presses,
    );
    let measurer = TextHost::new(ZeroTextBackend);
    let mut translator = EventTranslator::default();
    let mut press = |arena: &mut UiRuntime, at: Point| {
        arena.dispatch_event(
            &measurer,
            &mut translator,
            RawEvent::PointerDown(pointer_at(at)),
        );
        arena.dispatch_event(
            &measurer,
            &mut translator,
            RawEvent::PointerUp(pointer_at(at)),
        );
    };

    // A page is 87.5% of the 100px viewport.
    press(&mut arena, Point::new(96.0, 90.0));
    assert_eq!(arena.node(scroll).unwrap().scroll_offset.y, 87.5);
    assert_eq!(
        arena.pointer_capture_node(),
        None,
        "a track press is not a drag"
    );

    // The thumb now starts at 21.875, so a press above it pages back.
    press(&mut arena, Point::new(96.0, 5.0));
    assert_eq!(arena.node(scroll).unwrap().scroll_offset.y, 0.0);

    assert_eq!(
        *log.borrow(),
        vec![
            (ScrollSource::Scrollbar, ScrollPhase::Move, 87.5),
            (ScrollSource::Scrollbar, ScrollPhase::Move, 0.0),
        ]
    );
    assert_eq!(presses.get(), 0);
}

#[test]
fn a_press_beside_the_scrollbar_still_reaches_content() {
    let mut arena = UiRuntime::new();
    let presses = Rc::new(Cell::new(0));
    let scroll = scrollbar_fixture(
        &mut arena,
        &crate::scroll::ScrollController::new(),
        &ScrollLog::default(),
        &presses,
    );
    let measurer = TextHost::new(ZeroTextBackend);
    let mut translator = EventTranslator::default();

    arena.dispatch_event(
        &measurer,
        &mut translator,
        RawEvent::PointerDown(pointer_at(Point::new(50.0, 50.0))),
    );

    assert_eq!(presses.get(), 1);
    assert_eq!(arena.node(scroll).unwrap().scroll_offset.y, 0.0);
}

/// A captured pointer keeps its own cursor even once it has left the
/// capturing node — the same rule that keeps events aimed there.
#[test]
fn a_captured_pointer_keeps_its_own_cursor() {
    let mut arena = UiRuntime::new();
    let grabber = create_host(
        &mut arena,
        WidgetI::new(
            container()
                .style(Style::new().size(Size::fix(50.0, 50.0)))
                .cursor(CursorIcon::Grabbing)
                .on_press_start(|_, cx| {
                    cx.capture_pointer();
                }),
        ),
    );
    let elsewhere = create_host(
        &mut arena,
        WidgetI::new(
            container()
                .style(Style::new().size(Size::fix(50.0, 50.0)))
                .cursor(CursorIcon::Text),
        ),
    );

    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(200.0, 200.0), &mut measurer);

    let centre = |arena: &UiRuntime, node: NodeId| {
        let bounds = arena.node(node).unwrap().layout;
        Point::new(
            (bounds.min.x + bounds.max.x) * 0.5,
            (bounds.min.y + bounds.max.y) * 0.5,
        )
    };
    let on_grabber = centre(&arena, grabber);
    let on_elsewhere = centre(&arena, elsewhere);

    let mut translator =
        EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());
    arena.dispatch_event(&measurer, &mut translator, pointer_move(on_grabber));
    assert_eq!(arena.resolved_cursor(), CursorIcon::Grabbing);

    arena.dispatch_event(
        &measurer,
        &mut translator,
        RawEvent::PointerDown(pointer_at(on_grabber)),
    );
    arena.dispatch_event(&measurer, &mut translator, pointer_move(on_elsewhere));

    assert_eq!(
        arena.resolved_cursor(),
        CursorIcon::Grabbing,
        "the cursor followed hit testing instead of the capture"
    );
}

/// A widget's own default reaches the platform without the application
/// asking, and stays overridable.
#[test]
fn a_widget_default_cursor_applies_and_can_be_overridden() {
    let mut arena = UiRuntime::new();
    let input = create_host(
        &mut arena,
        WidgetI::new(text_input().style(Style::new().size(Size::fix(80.0, 30.0)))),
    );
    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(200.0, 200.0), &mut measurer);

    let bounds = arena.node(input).unwrap().layout;
    let inside = Point::new(
        (bounds.min.x + bounds.max.x) * 0.5,
        (bounds.min.y + bounds.max.y) * 0.5,
    );
    let mut translator =
        EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());
    arena.dispatch_event(&measurer, &mut translator, pointer_move(inside));
    assert_eq!(arena.resolved_cursor(), CursorIcon::Text);

    let mut overridden = UiRuntime::new();
    let node = create_host(
        &mut overridden,
        WidgetI::new(
            text_input().style(
                Style::new()
                    .size(Size::fix(80.0, 30.0))
                    .cursor(CursorIcon::NotAllowed),
            ),
        ),
    );
    overridden.update_tree(Size::new(200.0, 200.0), &mut measurer);
    let bounds = overridden.node(node).unwrap().layout;
    let inside = Point::new(
        (bounds.min.x + bounds.max.x) * 0.5,
        (bounds.min.y + bounds.max.y) * 0.5,
    );
    let mut translator =
        EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());
    overridden.dispatch_event(&measurer, &mut translator, pointer_move(inside));
    assert_eq!(overridden.resolved_cursor(), CursorIcon::NotAllowed);
}

fn child_of(arena: &mut UiRuntime, parent: NodeId, widget: WidgetI) -> NodeId {
    let key = widget.key();
    let props_hash = widget.props_hash();
    let interaction = widget.take_host_interaction();
    let id = arena.create_node(key, props_hash, widget, interaction);
    arena.place(parent, id, None);
    id
}

/// A pane that fills the leftover height and scrolls, above a pane sized by
/// its content.
///
/// The canonical app shell, and the layout most likely to break: it depends
/// on `Sizing::Fill` becoming `flex_grow` on the main axis, on the sibling
/// staying content-sized, and on the scrolling pane being allowed to be
/// shorter than its content. The last one is why `min_size` is pinned to
/// zero rather than taffy's `auto` — see `taffy_style_for_widget`.
const WRAPPING_PARAGRAPH: &str = "The quick brown fox jumps over the lazy dog again and again";

fn active_text_size(measurer: &TextHost<xui_cosmic::CosmicEngine>, id: NodeId) -> Size<f32> {
    measurer
        .active_slot(id, TextLayoutSlot::PRIMARY)
        .and_then(|handle| measurer.layout(handle))
        .expect("final text layout must be active")
        .size()
}

/// A column shorter than its content used to crush every content-sized
/// child: three paragraphs shaped 42pt tall got 7pt boxes and painted over
/// one another. They now keep their height and overflow the column.
#[test]
fn wrapped_text_keeps_its_height_in_a_column_too_short_for_it() {
    let mut arena = UiRuntime::new();
    let column = create_host(
        &mut arena,
        WidgetI::new(
            container()
                .flex_direction(FlexDirectionStyle::Column)
                .style(Style::new().width(120.0).height(40.0)),
        ),
    );
    let texts: Vec<_> = (0..3)
        .map(|_| {
            child_of(
                &mut arena,
                column,
                WidgetI::new(TextWidget::new(WRAPPING_PARAGRAPH)),
            )
        })
        .collect();
    let mut measurer = TextHost::new(xui_cosmic::CosmicEngine::new(1.0));
    arena.update_tree(Size::new(400.0, 400.0), &mut measurer);

    assert_eq!(arena.node(column).unwrap().layout.height(), 40.0);
    let mut previous_bottom = 0.0;
    for id in texts {
        let rect = arena.node(id).unwrap().layout;
        let shaped = active_text_size(&measurer, id);
        assert!(shaped.height > 20.0, "the paragraph must wrap: {shaped:?}");
        assert!(
            (rect.height() - shaped.height).abs() < 0.01,
            "text box {rect:?} is not as tall as its shaped paragraph {shaped:?}"
        );
        assert!(
            rect.min.y >= previous_bottom - 0.01,
            "text box {rect:?} overlaps the sibling above it ending at {previous_bottom}"
        );
        previous_bottom = rect.max.y;
    }
}

/// Taffy measures a paragraph against its content box; the painted variant
/// used to wrap at the border-box width and so disagreed with its own box.
#[test]
fn padded_text_wraps_at_its_content_width() {
    let mut arena = UiRuntime::new();
    let column = create_host(
        &mut arena,
        WidgetI::new(
            container()
                .flex_direction(FlexDirectionStyle::Column)
                .style(Style::new().width(120.0)),
        ),
    );
    let text = child_of(
        &mut arena,
        column,
        WidgetI::new(
            TextWidget::new(WRAPPING_PARAGRAPH)
                .style(Style::new().padding(xui_interface::EdgeInsets::symmetric(16.0, 4.0))),
        ),
    );
    let mut measurer = TextHost::new(xui_cosmic::CosmicEngine::new(1.0));
    arena.update_tree(Size::new(400.0, 400.0), &mut measurer);

    let rect = arena.node(text).unwrap().layout;
    let shaped = active_text_size(&measurer, text);
    assert!(
        shaped.width <= rect.width() - 32.0 + 0.01,
        "paragraph {shaped:?} is wider than the content box of {rect:?}"
    );
    assert!(
        (rect.height() - 8.0 - shaped.height).abs() < 0.01,
        "text box {rect:?} does not fit its shaped paragraph {shaped:?} plus padding"
    );
}

fn build_two_pane_shell(scrollable: bool) -> (UiRuntime, NodeId, NodeId, NodeId) {
    let mut arena = UiRuntime::new();
    let vcol = || container().flex_direction(FlexDirectionStyle::Column);

    let outer = create_host(
        &mut arena,
        WidgetI::new(vcol().style(Style::new().width(Sizing::Fill).height(Sizing::Fill))),
    );

    let mut top = Style::new().width(Sizing::Fill).height(Sizing::Fill);
    if scrollable {
        top = top.scroll_vertical();
    }
    let filling = child_of(&mut arena, outer, WidgetI::new(vcol().style(top)));
    // 500 of content in a pane that can only be 240 tall.
    for _ in 0..5 {
        child_of(
            &mut arena,
            filling,
            WidgetI::new(container().style(Style::new().width(Sizing::Fill).height(100.0))),
        );
    }

    let hugging = child_of(
        &mut arena,
        outer,
        WidgetI::new(vcol().style(Style::new().width(Sizing::Fill))),
    );
    child_of(
        &mut arena,
        hugging,
        WidgetI::new(container().style(Style::new().width(Sizing::Fill).height(60.0))),
    );

    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(400.0, 300.0), &mut measurer);
    (arena, outer, filling, hugging)
}

#[test]
fn a_filling_scroll_pane_shares_the_viewport_with_a_content_sized_sibling() {
    let (arena, outer, filling, hugging) = build_two_pane_shell(true);
    let height = |id: NodeId| arena.node(id).unwrap().layout.height();

    assert_eq!(
        height(outer),
        300.0,
        "the shell must not exceed the viewport"
    );
    assert_eq!(
        height(hugging),
        60.0,
        "the bottom pane is sized by its content"
    );
    assert_eq!(
        height(filling),
        240.0,
        "the top pane takes exactly what is left"
    );
    assert_eq!(
        arena.node(filling).unwrap().content_size.height,
        500.0,
        "the pane must still know how tall its content is, or it cannot scroll"
    );
}

/// Without a scroller, the filling pane is still capped at the space it was
/// given; its content overflows rather than pushing the shell past the
/// viewport.
///
/// This is what `min_size: ZERO` buys. With taffy's default of `auto`, the
/// automatic minimum size of a flex item is its content, so this same tree
/// lays out 560 tall inside a 300 tall window.
#[test]
fn a_filling_pane_is_capped_even_when_its_content_does_not_fit() {
    let (arena, outer, filling, hugging) = build_two_pane_shell(false);
    let height = |id: NodeId| arena.node(id).unwrap().layout.height();

    assert_eq!(height(outer), 300.0);
    assert_eq!(height(filling), 240.0);
    assert_eq!(height(hugging), 60.0);
}

/// The counter that lets raw dispatch skip its ancestor walk. If it ever
/// drifts, raw events stop reaching the widgets that need them — silently —
/// so it is checked against node creation and removal directly.
#[test]
fn the_raw_listener_count_tracks_node_lifetime() {
    let mut arena = UiRuntime::new();
    assert!(
        !arena.has_raw_event_listeners(),
        "an empty tree has nothing reading raw events"
    );

    let plain = create_host(&mut arena, WidgetI::new(container()));
    assert!(!arena.has_raw_event_listeners());
    assert!(!arena.node_reads_raw_events(plain));

    let input = create_host(&mut arena, WidgetI::new(text_input()));
    assert!(arena.has_raw_event_listeners());
    assert!(arena.node_reads_raw_events(input));

    arena.remove_subtree(plain);
    assert!(
        arena.has_raw_event_listeners(),
        "removing an unrelated node dropped the count"
    );

    arena.remove_subtree(input);
    assert!(!arena.has_raw_event_listeners());
}

/// Capture steers *both* dispatch layers.
///
/// The runtime and the translator used to keep separate pointer-capture
/// state, and only the runtime's was ever written to — so raw events
/// followed the capture while semantic events silently hit-tested. They now
/// resolve through the same one.
#[test]
fn a_captured_pointer_keeps_aiming_at_the_capturing_node() {
    let hovered_sibling = Rc::new(Cell::new(false));
    let flag = Rc::clone(&hovered_sibling);

    let mut arena = UiRuntime::new();
    let grabber = create_host(
        &mut arena,
        WidgetI::new(
            container()
                .style(Style::new().size(Size::fix(50.0, 50.0)))
                .on_press_start(|_, cx| {
                    cx.capture_pointer();
                }),
        ),
    );
    let sibling = create_host(
        &mut arena,
        WidgetI::new(
            container()
                .style(Style::new().size(Size::fix(50.0, 50.0)))
                .on_hovered(move |event, _| {
                    if event.hovered {
                        flag.set(true);
                    }
                }),
        ),
    );

    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(200.0, 200.0), &mut measurer);

    let centre = |arena: &UiRuntime, node: NodeId| {
        let bounds = arena.node(node).unwrap().layout;
        Point::new(
            (bounds.min.x + bounds.max.x) * 0.5,
            (bounds.min.y + bounds.max.y) * 0.5,
        )
    };
    let on_grabber = centre(&arena, grabber);
    let on_sibling = centre(&arena, sibling);
    assert_ne!(on_grabber, on_sibling, "the two nodes must not overlap");

    let mut translator =
        EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());
    arena.dispatch_event(&measurer, &mut translator, pointer_move(on_grabber));
    arena.dispatch_event(
        &measurer,
        &mut translator,
        RawEvent::PointerDown(pointer_at(on_grabber)),
    );
    assert_eq!(
        arena.pointer_capture_node(),
        Some(grabber),
        "the press handler did not take the capture"
    );

    arena.dispatch_event(&measurer, &mut translator, pointer_move(on_sibling));

    assert!(
        !hovered_sibling.get(),
        "the pointer was captured, but the semantic layer still hit-tested \
         its way onto the sibling"
    );
}

/// A handler that returns nothing at all is the common case; it used to have
/// to spell out `EventResult::Ignored`.
#[test]
fn a_handler_may_return_unit_flow_or_the_old_event_result() {
    let mut arena = UiRuntime::new();
    let node = create_host(
        &mut arena,
        WidgetI::new(
            container()
                .style(Style::new().size(Size::fix(40.0, 40.0)))
                .on_click(|_, _| {})
                .on_press_start(|_, _| Flow::empty())
                .on_hovered(|_, _| EventResult::Ignored),
        ),
    );
    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(200.0, 200.0), &mut measurer);
    click(&mut arena, &measurer, node);
    assert!(arena.contains(node));
}

/// `STOP_PROPAGATION` keeps an ancestor from seeing the event; on its own it
/// says nothing about the widget's own behaviour.
#[test]
fn stopping_propagation_hides_the_event_from_ancestors() {
    let seen_by_parent = Rc::new(Cell::new(false));
    let mut arena = UiRuntime::new();

    let parent_flag = Rc::clone(&seen_by_parent);
    let parent = create_host(
        &mut arena,
        WidgetI::new(
            container()
                .style(Style::new().size(Size::fix(80.0, 80.0)))
                .on_click(move |_, _| parent_flag.set(true)),
        ),
    );
    let child = {
        let widget = WidgetI::new(
            container()
                .style(Style::new().size(Size::fix(40.0, 40.0)))
                .on_click(|_, _| Flow::STOP_PROPAGATION),
        );
        let key = widget.key();
        let props_hash = widget.props_hash();
        let interaction = widget.take_host_interaction();
        let id = arena.create_node(key, props_hash, widget, interaction);
        arena.place(parent, id, None);
        id
    };

    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(200.0, 200.0), &mut measurer);
    click(&mut arena, &measurer, child);

    assert!(
        !seen_by_parent.get(),
        "the click bubbled past a handler that stopped it"
    );
}

fn pointer_move(position: Point) -> RawEvent {
    RawEvent::PointerMove(RawPointerMove {
        position,
        pointer_id: XuiPointerId::new(0),
        device_id: None,
        kind: PointerKind::Mouse,
        button: None,
        buttons: PointerButtons::default(),
        modifiers: Modifiers::default(),
        timestamp: Instant::now(),
    })
}

#[test]
fn host_metadata_binds_focus_handle_and_preserves_accessibility() {
    let mut arena = UiRuntime::new();
    let handle = FocusHandle::new();
    let widget = WidgetI::new(
        container()
            .focusable(true)
            .tab_index(-1)
            .focus_handle(handle.clone())
            .accessibility_role(xui_interface::AccessibilityRole::Tab)
            .accessibility_id("settings-tab")
            .accessibility_label("Settings")
            .accessibility_selected(true)
            .accessibility_controls("settings-panel"),
    );
    let key = widget.key();
    let props_hash = widget.props_hash();
    let handlers = widget.take_host_interaction();
    let node = arena.create_node(key, props_hash, widget, handlers);
    arena.place(arena.root(), node, None);

    assert_eq!(handle.node_id(), Some(node));
    assert!(arena.is_focusable(node));
    assert!(!arena.is_sequentially_focusable(node));
    assert_eq!(
        arena.accessibility(node).unwrap().role,
        Some(xui_interface::AccessibilityRole::Tab)
    );
    assert_eq!(
        arena.accessibility(node).unwrap().controls.as_deref(),
        Some("settings-panel")
    );

    arena.remove_subtree(node);
    assert!(!handle.is_bound());
}

#[test]
fn subtree_removal_clears_all_subsystem_caches() {
    let mut arena = UiRuntime::new();
    let node = create_host(
        &mut arena,
        WidgetI::new(
            container()
                .focusable(true)
                .on_click(|_, _| EventResult::Ignored),
        ),
    );

    assert!(arena.hosts.contains_key(node));
    assert!(arena.style_system.contains(node));
    assert!(arena.layout_tree.contains(node));
    assert!(arena.interaction_system.get(node).is_some());
    assert!(arena.render_system.has_binding(node));

    arena.remove_subtree(node);

    assert!(!arena.hosts.contains_key(node));
    assert!(!arena.style_system.contains(node));
    assert!(!arena.layout_tree.contains(node));
    assert!(arena.interaction_system.get(node).is_none());
    assert!(!arena.render_system.has_binding(node));
}

#[test]
fn interaction_cache_remains_sparse_for_default_widgets() {
    let mut arena = UiRuntime::new();
    let node = create_host(&mut arena, WidgetI::new(container()));

    assert!(arena.interaction_system.get(node).is_none());
}

#[test]
fn focus_handle_can_request_focus_for_another_node() {
    let mut arena = UiRuntime::new();
    let target_handle = FocusHandle::new();

    let source_widget = WidgetI::new(container().focusable(true));
    let source_hash = source_widget.props_hash();
    let source_handlers = source_widget.take_host_interaction();
    let source = arena.create_node(None, source_hash, source_widget, source_handlers);
    arena.place(arena.root(), source, None);

    let target_widget = WidgetI::new(
        container()
            .focusable(true)
            .focus_handle(target_handle.clone()),
    );
    let target_hash = target_widget.props_hash();
    let target_handlers = target_widget.take_host_interaction();
    let target = arena.create_node(None, target_hash, target_widget, target_handlers);
    arena.place(arena.root(), target, None);

    let mut flags = WidgetUpdateFlags::empty();
    let mut requests = xui_interface::EventRequests::default();
    let mut cx = crate::event_system::EventContext::new(
        arena.node(source).unwrap(),
        None,
        xui_interface::EventPhase::Target,
        &mut flags,
        &mut requests,
    );
    assert!(target_handle.request_focus(&mut cx));
    assert_eq!(
        requests.iter().collect::<Vec<_>>(),
        vec![xui_interface::EventRequest::Focus(target)]
    );
}

#[test]
fn host_metadata_and_focus_handle_update_with_reused_node() {
    let mut arena = UiRuntime::new();
    let old_handle = FocusHandle::new();
    let initial = WidgetI::new(
        container()
            .tab_index(0)
            .focus_handle(old_handle.clone())
            .accessibility_role(xui_interface::AccessibilityRole::Tab)
            .accessibility_selected(false),
    );
    let initial_hash = initial.props_hash();
    let initial_handlers = initial.take_host_interaction();
    let node = arena.create_node(None, initial_hash, initial, initial_handlers);
    arena.place(arena.root(), node, None);

    let new_handle = FocusHandle::new();
    let updated = WidgetI::new(
        container()
            .tab_index(-1)
            .focus_handle(new_handle.clone())
            .accessibility_role(xui_interface::AccessibilityRole::Tab)
            .accessibility_selected(true),
    );
    let updated_hash = updated.props_hash();
    let updated_handlers = updated.take_host_interaction();
    arena.update_node(node, None, updated_hash, updated, updated_handlers);

    assert!(!old_handle.is_bound());
    assert_eq!(new_handle.node_id(), Some(node));
    assert_eq!(arena.tab_index(node), Some(-1));
    assert_eq!(arena.accessibility(node).unwrap().selected, Some(true));
}

#[test]
fn final_text_width_is_activated_after_layout_measurement() {
    let mut arena = UiRuntime::new();
    let node = create_host(
        &mut arena,
        WidgetI::new(TextWidget::new("飞行监测").style(Style::new().width(120.0))),
    );
    let mut measurer = TextHost::new(ZeroTextBackend);

    arena.update_tree(Size::new(400.0, 200.0), &mut measurer);

    let active = measurer
        .active_slot(node, TextLayoutSlot::PRIMARY)
        .expect("final layout must activate regular text");
    let host_node = arena.node(node).unwrap();
    assert_eq!(host_node.layout.width(), 120.0);
    let props = host_node
        .widget
        .with_widgets(|widget| widget.text_layout_props(&host_node.effective_style))
        .unwrap();
    let final_input = TextLayoutInput::new(
        props.text,
        TextLayoutConstraints::max_width(host_node.layout.width()),
        props.style.into(),
        props.paragraph,
        props.text_box,
        measurer.backend().epoch(),
    );

    let expected = measurer.activate_slot(node, TextLayoutSlot::PRIMARY, final_input);
    assert_eq!(active, expected);

    let host_node = arena.node(node).unwrap();
    let props = host_node
        .widget
        .with_widgets(|widget| widget.text_layout_props(&host_node.effective_style))
        .unwrap();
    let font_context = measurer.backend().epoch();
    measurer.measure_slot(
        node,
        TextLayoutSlot::PRIMARY,
        TextLayoutInput::new(
            props.text,
            TextLayoutConstraints::MIN_SIZE,
            props.style.into(),
            props.paragraph,
            props.text_box,
            font_context,
        ),
    );
    assert_eq!(
        measurer.active_slot(node, TextLayoutSlot::PRIMARY),
        Some(active)
    );
}

#[test]
fn mounted_text_input_has_an_active_layout_before_any_edit() {
    // A definite size means Taffy never runs the measure function, so the
    // paint layout must come from post-layout activation; without it the
    // renderer rejected the first frame for a missing text layout.
    let mut arena = UiRuntime::new();
    let node = create_host(
        &mut arena,
        WidgetI::new(text_input().style(Style::new().size(Size::fix(120.0, 30.0)))),
    );
    let mut measurer = TextHost::new(ZeroTextBackend);

    arena.update_tree(Size::new(400.0, 200.0), &mut measurer);

    assert!(
        measurer
            .active_slot(node, TextLayoutSlot::PRIMARY)
            .is_some()
    );
}

fn child_host(arena: &mut UiRuntime, parent: NodeId, widget: WidgetI) -> NodeId {
    let id = detached_host(arena, widget);
    arena.place(parent, id, None);
    id
}

fn detached_host(arena: &mut UiRuntime, widget: WidgetI) -> NodeId {
    let key = widget.key();
    let props_hash = widget.props_hash();
    let interaction = widget.take_host_interaction();
    arena.create_node(key, props_hash, widget, interaction)
}

/// Mounts `visual_root` as a Portal written under `owner`, as the
/// reconciler would.
fn mount_portal(
    arena: &mut UiRuntime,
    owner: NodeId,
    visual_root: NodeId,
    options: OverlayEntryOptions,
    behavior: crate::element::PortalBehavior,
) -> OverlayEntryId {
    let entry = arena
        .mount_overlay_entry(visual_root, None, options)
        .unwrap();
    arena
        .set_overlay_entry_dismiss(entry, behavior.on_dismiss)
        .unwrap();
    let target = (owner != arena.root()).then_some(owner);
    arena.set_anchor(
        visual_root,
        behavior.anchor.map(|placement| (target, placement)),
    );
    entry
}

fn anchored(placement: crate::anchor::AnchorPlacement) -> crate::element::PortalBehavior {
    crate::element::PortalBehavior {
        anchor: Some(placement),
        ..Default::default()
    }
}

fn sized(width: f32, height: f32) -> WidgetI {
    WidgetI::new(container().style(Style::new().size(Size::fix(width, height))))
}

#[test]
fn an_anchored_portal_follows_its_owner_across_a_resize() {
    use crate::anchor::AnchorPlacement;

    let mut arena = UiRuntime::new();
    // A centered 80x20 trigger: its x depends on the window width.
    let page = create_host(
        &mut arena,
        WidgetI::new(
            container()
                .flex_direction(xui_interface::FlexDirectionStyle::Row)
                .style(
                    Style::new()
                        .size(Size::fill())
                        .justify(xui_interface::JustifyStyle::Center)
                        .padding(xui_interface::EdgeInsets::new(0.0, 0.0, 30.0, 0.0)),
                ),
        ),
    );
    let trigger = child_host(&mut arena, page, sized(80.0, 20.0));
    let menu = detached_host(
        &mut arena,
        WidgetI::new(container().style(Style::new().height(50.0))),
    );
    mount_portal(
        &mut arena,
        trigger,
        menu,
        OverlayEntryOptions::default(),
        anchored(AnchorPlacement::default().match_width(true)),
    );

    let mut measurer = TextHost::new(ZeroTextBackend);
    let menu_bounds = |arena: &UiRuntime| arena.visual_layout(menu);
    for width in [200.0, 400.0] {
        arena.mark_subtree_layout_dirty(arena.root());
        arena.update_tree(Size::new(width, 300.0), &mut measurer);
        let trigger_bounds = arena.visual_layout(trigger);
        assert_eq!(trigger_bounds.min.x, (width - 80.0) / 2.0);
        let menu = menu_bounds(&arena);
        assert_eq!(menu.min.x, trigger_bounds.min.x, "at width {width}");
        assert_eq!(menu.min.y, trigger_bounds.max.y + 4.0, "at width {width}");
        assert_eq!(menu.width(), 80.0);
    }

    // The trigger moved but kept its width: placing the menu needs no
    // second Taffy pass.
    let passes = arena.stats.layout_passes;
    arena.mark_subtree_layout_dirty(arena.root());
    arena.update_tree(Size::new(300.0, 300.0), &mut measurer);
    assert_eq!(arena.stats.layout_passes, passes + 1);
    assert_eq!(menu_bounds(&arena).min, Point::new(110.0, 54.0));

    // Layout keeps the menu at its parent's origin; the anchor is paint.
    assert_eq!(arena.node(menu).unwrap().layout.origin(), Point::zero());
    let binding = arena.render_system.binding(menu).transform;
    assert_eq!(
        arena
            .render_system
            .properties
            .transform(binding)
            .unwrap()
            .value,
        Affine::translate(110.0, 54.0)
    );
    assert_eq!(arena.hit_test(Point::new(120.0, 60.0)), Some(menu));
}

#[test]
fn an_anchored_portal_follows_a_scroll_without_laying_out() {
    use crate::anchor::AnchorPlacement;

    let mut arena = UiRuntime::new();
    let scroll = create_host(
        &mut arena,
        WidgetI::new(container().style(Style::new().size(Size::fill()).scroll_vertical())),
    );
    let content = child_host(
        &mut arena,
        scroll,
        WidgetI::new(
            container().style(
                Style::new()
                    .width(Sizing::fill())
                    .height(900.0)
                    .padding(xui_interface::EdgeInsets::new(10.0, 0.0, 100.0, 0.0)),
            ),
        ),
    );
    let trigger = child_host(&mut arena, content, sized(80.0, 20.0));
    let menu = detached_host(&mut arena, sized(60.0, 40.0));
    mount_portal(
        &mut arena,
        trigger,
        menu,
        OverlayEntryOptions::default(),
        anchored(AnchorPlacement::default().offset(0.0)),
    );

    let mut measurer = TextHost::new(ZeroTextBackend);
    let viewport = Size::new(400.0, 300.0);
    arena.update_tree(viewport, &mut measurer);
    assert_eq!(arena.visual_layout(menu).min, Point::new(10.0, 120.0));

    let passes = arena.stats.layout_passes;
    assert!(arena.set_scroll_offset(scroll, Point::new(0.0, 40.0)));
    arena.update_tree(viewport, &mut measurer);
    assert_eq!(arena.stats.layout_passes, passes);
    assert_eq!(arena.visual_layout(menu).min, Point::new(10.0, 80.0));
    assert_eq!(arena.hit_test(Point::new(20.0, 90.0)), Some(menu));
}

#[test]
fn a_chained_anchor_is_placed_after_the_portal_it_depends_on() {
    use crate::anchor::{AnchorPlacement, AnchorSide};

    let mut arena = UiRuntime::new();
    let page = create_host(
        &mut arena,
        WidgetI::new(
            container().style(
                Style::new()
                    .size(Size::fill())
                    .padding(xui_interface::EdgeInsets::new(20.0, 0.0, 40.0, 0.0)),
            ),
        ),
    );
    let trigger = child_host(&mut arena, page, sized(80.0, 20.0));
    let menu = detached_host(
        &mut arena,
        WidgetI::new(container().style(Style::new().width(100.0))),
    );
    let item = child_host(&mut arena, menu, sized(100.0, 24.0));
    // Anchored roots are stored in hash order, so without dependency
    // ordering the submenu often reads the menu before it has moved.
    let submenu = detached_host(&mut arena, sized(60.0, 40.0));
    mount_portal(
        &mut arena,
        item,
        submenu,
        OverlayEntryOptions::default(),
        anchored(AnchorPlacement::new(AnchorSide::Right).offset(0.0)),
    );
    mount_portal(
        &mut arena,
        trigger,
        menu,
        OverlayEntryOptions::default(),
        anchored(AnchorPlacement::default().offset(0.0)),
    );

    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(400.0, 300.0), &mut measurer);

    let bounds = |id: NodeId| arena.visual_layout(id);
    assert_eq!(bounds(trigger).min, Point::new(20.0, 40.0));
    assert_eq!(bounds(menu).min, Point::new(20.0, 60.0));
    assert_eq!(bounds(item).min, Point::new(20.0, 60.0));
    assert_eq!(bounds(submenu).min, Point::new(120.0, 60.0));
    assert_eq!(arena.stats.layout_passes, 1);
}

#[test]
fn an_anchored_portal_flips_above_a_trigger_near_the_bottom() {
    use crate::anchor::AnchorPlacement;

    let mut arena = UiRuntime::new();
    let page = create_host(
        &mut arena,
        WidgetI::new(
            container().style(
                Style::new()
                    .size(Size::fill())
                    .padding(xui_interface::EdgeInsets::new(10.0, 0.0, 260.0, 0.0)),
            ),
        ),
    );
    let trigger = child_host(&mut arena, page, sized(80.0, 20.0));
    let menu = detached_host(&mut arena, sized(120.0, 100.0));
    mount_portal(
        &mut arena,
        trigger,
        menu,
        OverlayEntryOptions::default(),
        anchored(AnchorPlacement::default()),
    );

    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(400.0, 300.0), &mut measurer);

    let menu = arena.visual_layout(menu);
    assert_eq!(menu.min, Point::new(10.0, 260.0 - 4.0 - 100.0));
}

#[test]
fn dropping_the_anchor_returns_the_portal_to_its_own_position() {
    use crate::anchor::AnchorPlacement;

    let mut arena = UiRuntime::new();
    let page = create_host(
        &mut arena,
        WidgetI::new(
            container().style(
                Style::new()
                    .size(Size::fill())
                    .padding(xui_interface::EdgeInsets::new(40.0, 0.0, 40.0, 0.0)),
            ),
        ),
    );
    let trigger = child_host(&mut arena, page, sized(80.0, 20.0));
    let menu = detached_host(&mut arena, sized(60.0, 40.0));
    mount_portal(
        &mut arena,
        trigger,
        menu,
        OverlayEntryOptions::default(),
        anchored(AnchorPlacement::default()),
    );
    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(400.0, 300.0), &mut measurer);
    assert_eq!(arena.visual_layout(menu).min, Point::new(40.0, 64.0));

    arena.set_anchor(menu, None);
    arena.update_tree(Size::new(400.0, 300.0), &mut measurer);
    assert_eq!(arena.visual_layout(menu).min, Point::zero());
    let binding = arena.render_system.binding(menu).transform;
    assert!(arena.render_system.properties.transform(binding).is_none());
}

fn escape() -> RawEvent {
    use xui_interface::events::{KeyState, NamedKey, PhysicalKey, RawKeyboard};
    RawEvent::Keyboard(RawKeyboard {
        physical_key: PhysicalKey::Unidentified,
        named_key: Some(NamedKey::Escape),
        state: KeyState::Down,
        text: None,
        modifiers: Modifiers::default(),
        timestamp: Instant::now(),
        is_repeat: false,
    })
}

/// A dismissable 60x40 Portal at (100, 100), recording every dismissal.
fn dismissable_portal(
    arena: &mut UiRuntime,
    options: OverlayEntryOptions,
) -> (NodeId, Rc<RefCell<Vec<crate::widgets::DismissReason>>>) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let panel = detached_host(
        arena,
        WidgetI::new(
            container().style(
                Style::new()
                    .absolute()
                    .inset(xui_interface::EdgeInsets::new(100.0, 0.0, 100.0, 0.0))
                    .size(Size::fix(60.0, 40.0)),
            ),
        ),
    );
    let behavior = crate::element::PortalBehavior {
        on_dismiss: Some(crate::widgets::DismissHandler::new({
            let seen = Rc::clone(&seen);
            move |reason| seen.borrow_mut().push(reason)
        })),
        ..Default::default()
    };
    let root = arena.root();
    mount_portal(arena, root, panel, options, behavior);
    (panel, seen)
}

#[test]
fn a_press_outside_or_escape_dismisses_a_portal_and_a_press_inside_does_not() {
    use crate::widgets::DismissReason;

    let mut arena = UiRuntime::new();
    let (_, seen) = dismissable_portal(&mut arena, OverlayEntryOptions::default());
    // A layer painted below the dismissable one: pressing it is outside.
    let below = detached_host(&mut arena, sized(50.0, 50.0));
    let root = arena.root();
    mount_portal(
        &mut arena,
        root,
        below,
        OverlayEntryOptions {
            z_index: -1,
            ..Default::default()
        },
        Default::default(),
    );
    let measurer = TextHost::new(ZeroTextBackend);
    let mut text = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(400.0, 300.0), &mut text);
    let mut translator =
        EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());

    let press = |arena: &mut UiRuntime, translator: &mut EventTranslator, at| {
        arena.dispatch_event(&measurer, translator, RawEvent::PointerDown(pointer_at(at)));
    };
    press(&mut arena, &mut translator, Point::new(120.0, 120.0));
    assert!(
        seen.borrow().is_empty(),
        "a press inside is not a dismissal"
    );

    assert_eq!(arena.hit_test(Point::new(10.0, 10.0)), Some(below));
    press(&mut arena, &mut translator, Point::new(10.0, 10.0));
    assert_eq!(*seen.borrow(), [DismissReason::PointerOutside]);

    press(&mut arena, &mut translator, Point::new(300.0, 250.0));
    assert_eq!(
        *seen.borrow(),
        [DismissReason::PointerOutside, DismissReason::PointerOutside]
    );
    seen.borrow_mut().clear();

    let result = arena.dispatch_event(&measurer, &mut translator, escape());
    assert_eq!(result, EventResult::Consumed);
    assert_eq!(*seen.borrow(), [DismissReason::Escape]);
}

#[test]
fn a_modal_portal_without_a_handler_shields_the_portal_below_it() {
    let mut arena = UiRuntime::new();
    let (_, seen) = dismissable_portal(&mut arena, OverlayEntryOptions::default());
    let dialog = detached_host(&mut arena, sized(10.0, 10.0));
    let root = arena.root();
    mount_portal(
        &mut arena,
        root,
        dialog,
        OverlayEntryOptions {
            z_index: 10,
            modal: true,
            ..Default::default()
        },
        Default::default(),
    );
    let measurer = TextHost::new(ZeroTextBackend);
    let mut text = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(400.0, 300.0), &mut text);
    let mut translator =
        EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());

    arena.dispatch_event(
        &measurer,
        &mut translator,
        RawEvent::PointerDown(pointer_at(Point::new(300.0, 250.0))),
    );
    arena.dispatch_event(&measurer, &mut translator, escape());
    assert!(seen.borrow().is_empty());
}

#[test]
fn a_wrapping_row_moves_overflow_to_a_new_line_spaced_by_gap() {
    let mut arena = UiRuntime::new();
    let row = create_host(
        &mut arena,
        WidgetI::new(
            container()
                .flex_direction(xui_interface::FlexDirectionStyle::Row)
                .flex_wrap(true)
                .style(Style::new().width(100.0).gap(10.0)),
        ),
    );
    let items: Vec<NodeId> = (0..3)
        .map(|_| {
            let widget = WidgetI::new(container().style(Style::new().size(Size::fix(40.0, 20.0))));
            let key = widget.key();
            let props_hash = widget.props_hash();
            let interaction = widget.take_host_interaction();
            let id = arena.create_node(key, props_hash, widget, interaction);
            arena.place(row, id, None);
            id
        })
        .collect();
    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(400.0, 400.0), &mut measurer);

    let origin = |id: NodeId| {
        let layout = arena.node(id).unwrap().layout;
        (layout.x(), layout.y())
    };
    assert_eq!(origin(items[0]), (0.0, 0.0));
    assert_eq!(origin(items[1]), (50.0, 0.0));
    // 40 + 10 + 40 + 10 + 40 = 140 > 100: the third item wraps.
    assert_eq!(origin(items[2]), (0.0, 30.0));
    assert_eq!(arena.node(row).unwrap().layout.height(), 50.0);
}

#[test]
fn text_nodes_preserve_fractional_taffy_geometry() {
    let mut arena = UiRuntime::new();
    let node = create_host(
        &mut arena,
        WidgetI::new(TextWidget::new("中文").style(Style::new().width(45.5).height(20.0))),
    );
    let mut measurer = TextHost::new(ZeroTextBackend);

    arena.update_tree(Size::new(200.0, 100.0), &mut measurer);

    let rounded_width = arena.layout_tree.layout(node).size.width;
    let unrounded_width = arena.layout_tree.unrounded_layout(node).size.width;
    assert_eq!(rounded_width, 46.0);
    assert_eq!(unrounded_width, 45.5);
    assert_eq!(arena.node(node).unwrap().layout.width(), unrounded_width);
}

#[test]
fn resizing_invalidates_intrinsic_text_layout_caches() {
    fn create_child(arena: &mut UiRuntime, parent: NodeId, widget: WidgetI) -> NodeId {
        let child = create_host(arena, widget);
        arena.place(parent, child, None);
        child
    }

    let mut arena = UiRuntime::new();
    let outer = create_host(
        &mut arena,
        WidgetI::new(
            container()
                .flex_direction(xui_interface::FlexDirectionStyle::Row)
                .style(
                    Style::new()
                        .size(Size::fill())
                        .padding(xui_interface::EdgeInsets::all(16.0))
                        .gap(16.0),
                ),
        ),
    );
    create_child(
        &mut arena,
        outer,
        WidgetI::new(
            container().style(
                Style::new()
                    .width(xui_interface::Sizing::percent(0.4))
                    .height(xui_interface::Sizing::fill()),
            ),
        ),
    );
    let analytics = create_child(
        &mut arena,
        outer,
        WidgetI::new(
            container()
                .flex_direction(xui_interface::FlexDirectionStyle::Column)
                .style(
                    Style::new()
                        .size(Size::fill())
                        .padding(xui_interface::EdgeInsets::all(16.0))
                        .gap(12.0),
                ),
        ),
    );
    let tabs = create_child(
        &mut arena,
        analytics,
        WidgetI::new(
            container()
                .flex_direction(xui_interface::FlexDirectionStyle::Row)
                .style(
                    Style::new()
                        .gap(3.0)
                        .padding(xui_interface::EdgeInsets::all(4.0))
                        .border_width(1.0),
                ),
        ),
    );
    let tab = create_child(
        &mut arena,
        tabs,
        WidgetI::new(
            container().style(
                Style::new()
                    .padding(xui_interface::EdgeInsets::symmetric(16.0, 6.0))
                    .font_size(12.0)
                    .border_width(1.0),
            ),
        ),
    );
    let label = create_child(&mut arena, tab, WidgetI::new(TextWidget::new("飞行监测")));
    create_child(
        &mut arena,
        analytics,
        WidgetI::new(container().style(Style::new().size(Size::fill()))),
    );

    let mut measurer = TextHost::new(xui_cosmic::CosmicEngine::new(1.0));
    arena.update_tree(Size::new(1600.0, 900.0), &mut measurer);
    let expected_width = arena.node(label).unwrap().layout.width();
    assert!(expected_width > 12.0);

    for width in [900.0, 2000.0] {
        arena.mark_subtree_layout_dirty(arena.root());
        arena.update_tree(Size::new(width, 900.0), &mut measurer);

        let final_width = arena.node(label).unwrap().layout.width();
        let unrounded_width = arena.layout_tree.unrounded_layout(label).size.width;
        assert!(
            (final_width - expected_width).abs() < 0.01,
            "text width changed from {expected_width} to {final_width} after resizing to {width}"
        );
        assert_eq!(
            final_width, unrounded_width,
            "text layout must preserve its fractional intrinsic width"
        );
        let active = measurer
            .active_slot(label, TextLayoutSlot::PRIMARY)
            .and_then(|handle| measurer.layout(handle))
            .expect("final text layout must be active");
        assert!((active.size().width - final_width).abs() < 0.01);
        assert_eq!(
            active.lines.len(),
            1,
            "intrinsically-sized CJK text wrapped after resizing to {width}: rounded={:?}, unrounded={:?}, lines={:?}",
            arena.layout_tree.layout(label),
            arena.layout_tree.unrounded_layout(label),
            active.lines,
        );
    }
}

#[test]
fn local_paint_style_update_skips_unrelated_branch_and_layout() {
    let mut arena = UiRuntime::new();
    let left = create_host(
        &mut arena,
        WidgetI::new(container().style(Style::new().width(100.0).height(100.0))),
    );
    let right = create_host(
        &mut arena,
        WidgetI::new(container().style(Style::new().width(100.0).height(100.0))),
    );
    let left_leaf = create_host(
        &mut arena,
        WidgetI::new(container().style(Style::new().width(20.0).height(20.0))),
    );
    let right_leaf = create_host(
        &mut arena,
        WidgetI::new(container().style(Style::new().width(20.0).height(20.0))),
    );
    arena.place(left, left_leaf, None);
    arena.place(right, right_leaf, None);

    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(400.0, 200.0), &mut measurer);
    let layout_passes = arena.stats.layout_passes;
    let repaint_passes = arena.stats.repaint_passes;

    update_host(
        &mut arena,
        left_leaf,
        WidgetI::new(
            container().style(
                Style::new()
                    .width(20.0)
                    .height(20.0)
                    .background(Color::BLACK),
            ),
        ),
    );
    arena.update_tree(Size::new(400.0, 200.0), &mut measurer);

    assert_eq!(arena.stats.update_visits, 1);
    assert_eq!(arena.stats.layout_passes, layout_passes);
    assert_eq!(arena.stats.repaint_passes - repaint_passes, 1);
    assert_eq!(arena.node(right_leaf).unwrap().layout.width(), 20.0);
}

#[test]
fn state_style_changes_use_transition_owned_by_style() {
    let mut arena = UiRuntime::new();
    let transition = Transition::new(Duration::from_millis(100)).ease(Easing::Linear);
    let style = Style::new()
        .width(20.0)
        .height(20.0)
        .background(Color::BLACK)
        .when(WidgetState::HOVERED, |patch| patch.background(Color::WHITE))
        .transition(transition);
    let node = create_host(&mut arena, WidgetI::new(container().style(style)));
    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

    arena.set_widget_state_flag(node, WidgetState::HOVERED, true);
    arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
    assert!(arena.has_running_style_animations());

    arena.tick_style_animations(Duration::from_millis(50));
    let effective = arena.style_system.effective(node);
    let ComputedColorStyle::Solid(background) = effective.paint.background else {
        panic!("expected solid background")
    };
    assert!((background.r - 0.5).abs() < 0.0001);
    let layout_passes = arena.stats.layout_passes;
    let repaint_passes = arena.stats.repaint_passes;

    let style_without_transition = Style::new()
        .width(20.0)
        .height(20.0)
        .background(Color::BLACK)
        .when(WidgetState::HOVERED, |patch| patch.background(Color::WHITE));
    update_host(
        &mut arena,
        node,
        WidgetI::new(container().style(style_without_transition)),
    );
    arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

    assert!(!arena.has_running_style_animations());
    let effective = arena.style_system.effective(node);
    assert_eq!(
        effective.paint.background,
        ComputedColorStyle::Solid(Color::WHITE)
    );
    assert_eq!(arena.stats.layout_passes, layout_passes);
    assert_eq!(arena.stats.repaint_passes, repaint_passes + 1);
}

#[test]
fn inherited_text_color_follows_parent_transition_sample_without_layout() {
    let mut arena = UiRuntime::new();
    let transition = Transition::new(Duration::from_millis(100)).ease(Easing::Linear);
    let tab = create_host(
        &mut arena,
        WidgetI::new(
            container().style(
                Style::new()
                    .color(Color::BLACK)
                    .when(WidgetState::HOVERED, |patch| patch.color(Color::WHITE))
                    .transition(transition),
            ),
        ),
    );
    let label = create_host(&mut arena, WidgetI::new(TextWidget::new("New member")));
    let explicit_label = create_host(
        &mut arena,
        WidgetI::new(TextWidget::new("Pinned").style(Style::new().color(Color::BLUE_500))),
    );
    arena.place(tab, label, None);
    arena.place(tab, explicit_label, None);

    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(200.0, 100.0), &mut measurer);
    arena.set_widget_state_flag(tab, WidgetState::HOVERED, true);
    arena.update_tree(Size::new(200.0, 100.0), &mut measurer);
    let layout_passes = arena.stats.layout_passes;

    assert!(arena.tick_style_animations(Duration::from_millis(50)));
    arena.update_tree(Size::new(200.0, 100.0), &mut measurer);

    let parent_color = arena.style_system.effective(tab).text.color;
    let label_color = arena.style_system.effective(label).text.color;
    assert!((parent_color.r - 0.5).abs() < 0.0001);
    assert_eq!(label_color, parent_color);
    assert_eq!(
        arena.style_system.effective(explicit_label).text.color,
        Color::BLUE_500
    );
    assert_eq!(arena.stats.layout_passes, layout_passes);
}

/// A transition samples its `from` style on the frame it starts, so the
/// parent's effective color has not moved yet. Its children's targets must
/// still follow the parent's new target that frame, while what they show
/// keeps following the parent's sample.
#[test]
fn starting_an_inherited_transition_updates_child_targets_immediately() {
    let mut arena = UiRuntime::new();
    let transition = Transition::new(Duration::from_millis(100)).ease(Easing::Linear);
    let tab = create_host(
        &mut arena,
        WidgetI::new(
            container().style(
                Style::new()
                    .color(Color::BLACK)
                    .when(WidgetState::HOVERED, |patch| patch.color(Color::WHITE))
                    .transition(transition),
            ),
        ),
    );
    let label = create_host(&mut arena, WidgetI::new(TextWidget::new("New member")));
    arena.place(tab, label, None);

    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(200.0, 100.0), &mut measurer);
    arena.set_widget_state_flag(tab, WidgetState::HOVERED, true);
    arena.update_tree(Size::new(200.0, 100.0), &mut measurer);

    assert_eq!(arena.style_system.effective(tab).text.color, Color::BLACK);
    assert_eq!(
        arena.style_system.computed(label).text.color,
        Color::WHITE,
        "the child's target did not follow the parent's new target"
    );
    assert_eq!(
        arena.style_system.effective(label).text.color,
        Color::BLACK,
        "the child jumped ahead of the parent's transition"
    );
}

/// A tab whose hover transitions its text color, with an inheriting label
/// and a grandchild under it, and a label that sets its own color with a
/// grandchild under that.
fn inheritance_fixture(arena: &mut UiRuntime) -> [NodeId; 5] {
    let transition = Transition::new(Duration::from_millis(100)).ease(Easing::Linear);
    let tab = create_host(
        arena,
        WidgetI::new(
            container().style(
                Style::new()
                    .color(Color::BLACK)
                    .when(WidgetState::HOVERED, |patch| patch.color(Color::WHITE))
                    .transition(transition),
            ),
        ),
    );
    let label = create_host(arena, WidgetI::new(container()));
    let nested = create_host(arena, WidgetI::new(TextWidget::new("nested")));
    let pinned = create_host(
        arena,
        WidgetI::new(container().style(Style::new().color(Color::BLUE_500))),
    );
    let under_pinned = create_host(arena, WidgetI::new(TextWidget::new("under pinned")));
    arena.place(tab, label, None);
    arena.place(label, nested, None);
    arena.place(tab, pinned, None);
    arena.place(pinned, under_pinned, None);
    [tab, label, nested, pinned, under_pinned]
}

fn effective_color(arena: &UiRuntime, id: NodeId) -> Color {
    arena.style_system.effective(id).text.color
}

/// Pruning stops at a node whose inherited properties did not move, which
/// must still reach every inheriting descendant and leave the others alone.
#[test]
fn a_parent_sample_reaches_inheriting_descendants_only() {
    let mut arena = UiRuntime::new();
    let [tab, label, nested, pinned, under_pinned] = inheritance_fixture(&mut arena);
    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(200.0, 100.0), &mut measurer);
    arena.set_widget_state_flag(tab, WidgetState::HOVERED, true);
    arena.update_tree(Size::new(200.0, 100.0), &mut measurer);

    arena.tick_style_animations(Duration::from_millis(50));
    arena.update_tree(Size::new(200.0, 100.0), &mut measurer);

    let sample = effective_color(&arena, tab);
    assert!((sample.r - 0.5).abs() < 0.0001);
    assert_eq!(effective_color(&arena, label), sample);
    assert_eq!(effective_color(&arena, nested), sample);
    assert_eq!(effective_color(&arena, pinned), Color::BLUE_500);
    assert_eq!(effective_color(&arena, under_pinned), Color::BLUE_500);
    // The tab, its two children, and the one grandchild that inherits.
    assert!(
        arena.stats.inheritance_visits <= 4,
        "the sync walked past the pinned subtree: {} visits",
        arena.stats.inheritance_visits
    );
}

/// When the parent's transition ends its sample becomes its target, and
/// the samples it had pushed down must go with it.
#[test]
fn inherited_samples_are_dropped_when_the_parent_transition_ends() {
    let mut arena = UiRuntime::new();
    let [tab, label, nested, ..] = inheritance_fixture(&mut arena);
    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(200.0, 100.0), &mut measurer);
    arena.set_widget_state_flag(tab, WidgetState::HOVERED, true);
    arena.update_tree(Size::new(200.0, 100.0), &mut measurer);
    arena.tick_style_animations(Duration::from_millis(50));
    arena.update_tree(Size::new(200.0, 100.0), &mut measurer);
    assert!(arena.style_system.has_inherited_sample(nested));

    arena.tick_style_animations(Duration::from_millis(100));
    arena.update_tree(Size::new(200.0, 100.0), &mut measurer);

    assert!(!arena.has_running_style_animations());
    for id in [label, nested] {
        assert_eq!(effective_color(&arena, id), Color::WHITE);
        assert!(!arena.style_system.has_inherited_sample(id));
    }
}

/// A node mounted under a mid-transition parent is queued by its own first
/// style resolve, not by anything the parent does.
#[test]
fn a_node_mounted_mid_transition_shows_the_parent_sample() {
    let mut arena = UiRuntime::new();
    let [tab, ..] = inheritance_fixture(&mut arena);
    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(200.0, 100.0), &mut measurer);
    arena.set_widget_state_flag(tab, WidgetState::HOVERED, true);
    arena.update_tree(Size::new(200.0, 100.0), &mut measurer);
    arena.tick_style_animations(Duration::from_millis(50));
    arena.update_tree(Size::new(200.0, 100.0), &mut measurer);

    let late = create_host(&mut arena, WidgetI::new(TextWidget::new("late")));
    arena.place(tab, late, None);
    arena.update_tree(Size::new(200.0, 100.0), &mut measurer);

    assert_eq!(arena.style_system.computed(late).text.color, Color::WHITE);
    assert_eq!(effective_color(&arena, late), effective_color(&arena, tab));
}

/// `diff` ignores `cursor`, which must keep it from dirtying anything but
/// not from reaching the computed style the cursor resolver reads.
#[test]
fn a_cursor_only_state_change_updates_the_target_without_repainting() {
    let mut arena = UiRuntime::new();
    let node = create_host(
        &mut arena,
        WidgetI::new(
            container().style(
                Style::new()
                    .size(Size::fix(20.0, 20.0))
                    .when(WidgetState::HOVERED, |patch| {
                        patch.cursor(CursorIcon::Pointer)
                    }),
            ),
        ),
    );
    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
    let layout_passes = arena.stats.layout_passes;
    let repaint_passes = arena.stats.repaint_passes;

    arena.set_widget_state_flag(node, WidgetState::HOVERED, true);
    arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

    assert_eq!(
        arena.style_system.computed(node).cursor,
        Some(CursorIcon::Pointer)
    );
    assert_eq!(arena.stats.layout_passes, layout_passes);
    assert_eq!(arena.stats.repaint_passes, repaint_passes);
}

#[test]
fn hovered_cjk_tab_keeps_one_line_and_finishes_paint_transition() {
    let mut arena = UiRuntime::new();
    let transition = Transition::new(Duration::from_millis(100)).ease(Easing::Linear);
    let tab = create_host(
        &mut arena,
        WidgetI::new(
            container().style(
                Style::new()
                    .padding(xui_interface::EdgeInsets::symmetric(16.0, 6.0))
                    .color(Color::BLACK)
                    .font_family("PingFang SC")
                    .font_size(12.0)
                    .border_width(1.0)
                    .when(WidgetState::HOVERED, |patch| {
                        patch.background(Color::BLACK).color(Color::WHITE)
                    })
                    .transition(transition),
            ),
        ),
    );
    let label = create_host(&mut arena, WidgetI::new(TextWidget::new("飞行监测")));
    arena.place(tab, label, None);

    let size = Size::new(400.0, 200.0);
    let mut measurer = TextHost::new(xui_cosmic::CosmicEngine::new(1.0));
    arena.update_tree(size, &mut measurer);
    let layout_passes = arena.stats.layout_passes;
    let tab_bounds = arena.node(tab).unwrap().layout;
    let pointer = Point::new(
        (tab_bounds.min.x + tab_bounds.max.x) * 0.5,
        (tab_bounds.min.y + tab_bounds.max.y) * 0.5,
    );
    let mut translator =
        EventTranslator::new(crate::event_system::translator::EventTranslatorConfig::default());

    arena.dispatch_event(&measurer, &mut translator, pointer_move(pointer));
    arena.update_tree(size, &mut measurer);
    assert!(
        arena
            .node(tab)
            .unwrap()
            .state
            .contains(WidgetState::HOVERED)
    );
    assert!(arena.has_running_style_animations());
    assert_eq!(arena.stats.layout_passes, layout_passes);

    for frame in 0..20 {
        // Repeated cursor notifications at a stationary position must not
        // toggle the ancestor hover state or restart its transition.
        arena.dispatch_event(&measurer, &mut translator, pointer_move(pointer));
        assert!(
            arena.ui_state.layout_dirty_list.is_empty(),
            "pointer dispatch dirtied layout on frame {frame}"
        );
        arena.tick_style_animations(Duration::from_millis(8));
        assert!(
            arena.ui_state.layout_dirty_list.is_empty(),
            "animation tick dirtied layout on frame {frame}"
        );
        arena.update_tree(size, &mut measurer);

        assert!(
            arena
                .node(tab)
                .unwrap()
                .state
                .contains(WidgetState::HOVERED)
        );
        let active = measurer
            .active_slot(label, TextLayoutSlot::PRIMARY)
            .and_then(|handle| measurer.layout(handle))
            .expect("hovered tab label must retain an active layout");
        assert_eq!(active.lines.len(), 1, "hover animation wrapped CJK label");
        assert_eq!(
            arena.stats.layout_passes, layout_passes,
            "paint-only hover transition triggered layout on frame {frame}"
        );
    }

    assert!(!arena.has_running_style_animations());
}

#[test]
fn layout_transition_updates_effective_taffy_style_each_frame() {
    let mut arena = UiRuntime::new();
    let transition = Transition::new(Duration::from_millis(100)).ease(Easing::Linear);
    let node = create_host(
        &mut arena,
        WidgetI::new(
            container().style(Style::new().width(20.0).height(20.0).transition(transition)),
        ),
    );
    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(200.0, 100.0), &mut measurer);

    update_host(
        &mut arena,
        node,
        WidgetI::new(
            container().style(
                Style::new()
                    .width(100.0)
                    .height(20.0)
                    .transition(transition),
            ),
        ),
    );
    arena.update_tree(Size::new(200.0, 100.0), &mut measurer);
    let layout_passes = arena.stats.layout_passes;

    assert!(arena.tick_style_animations(Duration::from_millis(50)));
    arena.update_tree(Size::new(200.0, 100.0), &mut measurer);

    let effective = arena.style_system.effective(node);
    let xui_interface::Sizing::Fix(width) = effective.layout.width else {
        panic!("expected fixed animated width")
    };
    assert!((width.into_inner() - 60.0).abs() < 0.0001);
    assert!((arena.node(node).unwrap().layout.width() - 60.0).abs() < 0.0001);
    assert_eq!(arena.stats.layout_passes, layout_passes + 1);
}

#[test]
fn paint_only_transition_does_not_run_layout() {
    let mut arena = UiRuntime::new();
    let transition = Transition::new(Duration::from_millis(100)).ease(Easing::Linear);
    let node = create_host(
        &mut arena,
        WidgetI::new(
            container().style(
                Style::new()
                    .width(20.0)
                    .height(20.0)
                    .background(Color::BLACK)
                    .transition(transition),
            ),
        ),
    );
    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

    update_host(
        &mut arena,
        node,
        WidgetI::new(
            container().style(
                Style::new()
                    .width(20.0)
                    .height(20.0)
                    .background(Color::WHITE)
                    .transition(transition),
            ),
        ),
    );
    arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
    let layout_passes = arena.stats.layout_passes;

    assert!(arena.tick_style_animations(Duration::from_millis(50)));
    arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

    assert_eq!(arena.stats.layout_passes, layout_passes);
}

#[test]
fn state_transform_transition_updates_only_frame_properties() {
    let mut arena = UiRuntime::new();
    let transition = Transition::new(Duration::from_millis(100)).ease(Easing::QuadIn);
    let style = Style::new()
        .width(20.0)
        .height(20.0)
        .when(WidgetState::PRESSED, |patch| patch.translate_y(4.0))
        .transition(transition);
    let node = create_host(&mut arena, WidgetI::new(container().style(style)));
    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

    arena.set_widget_state_flag(node, WidgetState::PRESSED, true);
    arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
    let layout_passes = arena.stats.layout_passes;
    let repaint_passes = arena.stats.repaint_passes;
    let transform_node = arena.render_system.binding(node).transform;
    assert!(
        arena
            .render_system
            .properties
            .transform(transform_node)
            .is_none()
    );

    assert!(arena.tick_style_animations(Duration::from_millis(50)));
    let effective = arena.style_system.effective(node);
    assert!((effective.transform.translate.y - 1.0).abs() < 0.0001);
    // The tick only schedules the transform; the render sync applies it.
    assert!(
        arena
            .render_system
            .properties
            .transform(transform_node)
            .is_none()
    );

    arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
    assert_eq!(
        arena
            .render_system
            .properties
            .transform(transform_node)
            .unwrap()
            .value,
        Affine::translate(0.0, 1.0)
    );
    assert_eq!(arena.stats.layout_passes, layout_passes);
    assert_eq!(arena.stats.repaint_passes, repaint_passes);
}

#[test]
fn a_transform_is_derived_once_a_frame_from_the_settled_size() {
    let mut arena = UiRuntime::new();
    let node = create_host(
        &mut arena,
        WidgetI::new(container().style(Style::new().width(20.0).height(20.0))),
    );
    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

    // One update moves both inputs a scale about the center reads: the style
    // transform, and the size its origin is a fraction of.
    update_host(
        &mut arena,
        node,
        WidgetI::new(container().style(Style::new().width(40.0).height(20.0).scale(2.0))),
    );
    let syncs = arena.stats.transform_syncs;
    arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

    assert_eq!(arena.stats.transform_syncs, syncs + 1);
    let transform = arena.style_system.effective(node).transform;
    let transform_node = arena.render_system.binding(node).transform;
    assert_eq!(
        arena
            .render_system
            .properties
            .transform(transform_node)
            .unwrap()
            .value,
        transform.to_affine(Size::new(40.0, 20.0)),
        "the transform must be derived from the size layout just settled"
    );
}

#[test]
fn direct_transform_update_skips_layout_and_repaint() {
    let mut arena = UiRuntime::new();
    let node = create_host(
        &mut arena,
        WidgetI::new(container().style(Style::new().width(20.0).height(20.0))),
    );
    let mut measurer = TextHost::new(ZeroTextBackend);
    arena.update_tree(Size::new(100.0, 100.0), &mut measurer);
    let layout_passes = arena.stats.layout_passes;
    let repaint_passes = arena.stats.repaint_passes;

    update_host(
        &mut arena,
        node,
        WidgetI::new(
            container().style(
                Style::new()
                    .width(20.0)
                    .height(20.0)
                    .translate(Point::new(3.0, 5.0)),
            ),
        ),
    );
    arena.update_tree(Size::new(100.0, 100.0), &mut measurer);

    let transform_node = arena.render_system.binding(node).transform;
    assert_eq!(
        arena
            .render_system
            .properties
            .transform(transform_node)
            .unwrap()
            .value,
        Affine::translate(3.0, 5.0)
    );
    assert_eq!(arena.stats.layout_passes, layout_passes);
    assert_eq!(arena.stats.repaint_passes, repaint_passes);
}
