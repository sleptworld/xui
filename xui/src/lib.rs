pub mod app;
pub mod component;
pub mod core;
pub mod event;
pub mod fiber;
pub mod font;
pub mod lanes;
pub mod layout;
pub mod prelude;
pub mod render;
pub mod runtime;
pub mod state;
pub mod tree;
pub mod widgets;
pub use crate::prelude::*;
pub use xui_components as components;

// #[cfg(test)]
// mod component_runtime_tests {
//     use std::cell::Cell;
//     use std::rc::Rc;

//     use super::*;

//     fn click(app: &mut App, node: NodeId) {
//         let rect = app.arena().node(node).unwrap().layout;
//         let point = Point::new(rect.x + 2.0, rect.y + 2.0);
//         app.dispatch_event(Event::PointerDown {
//             position: point,
//             button: PointerButton::Primary,
//         });
//         app.dispatch_event(Event::PointerUp {
//             position: point,
//             button: PointerButton::Primary,
//         });
//     }

//     #[test]
//     fn component_state_rerenders_only_owner_component() {
//         let root_renders = Rc::new(Cell::new(0));
//         let child_renders = Rc::new(Cell::new(0));
//         let root_renders_for_app = root_renders.clone();
//         let child_renders_for_app = child_renders.clone();

//         let mut app = app(move |_| {
//             root_renders_for_app.set(root_renders_for_app.get() + 1);
//             let child_renders_for_component = child_renders_for_app.clone();
//             column()
//                 .child(component(move |cx| {
//                     child_renders_for_component.set(child_renders_for_component.get() + 1);
//                     let count = cx.use_state(|| 0);
//                     let count_for_click = count.clone();
//                     button(format!("count: {}", count.get()))
//                         .on_click(move || count_for_click.set(count_for_click.get() + 1))
//                         .into()
//                 }))
//                 .into()
//         });

//         let mut backend = MockRenderBackend::default();
//         app.resize(Size::new(240.0, 120.0));
//         app.render(&mut backend).unwrap();

//         assert_eq!(root_renders.get(), 1);
//         assert_eq!(child_renders.get(), 1);

//         let root = app.arena().root();
//         let column_id = app.arena().children(root)[0];
//         let button_id = app.arena().children(column_id)[0];
//         click(&mut app, button_id);
//         app.render(&mut backend).unwrap();

//         assert_eq!(root_renders.get(), 1);
//         assert_eq!(child_renders.get(), 2);
//         assert!(matches!(
//             &app.arena().node(button_id).unwrap().kind,
//             xui_interface::WidgetKind::Button { text, .. } if text == "count: 1"
//         ));
//     }

//     #[test]
//     fn keyed_component_reorder_preserves_hook_state() {
//         let reversed = Rc::new(Cell::new(false));
//         let reversed_for_app = reversed.clone();

//         let mut app = app(move |_| {
//             let first = component(|cx| {
//                 let value = cx.use_state(|| "A".to_owned());
//                 label(value.get()).into()
//             })
//             .key("a");
//             let second = component(|cx| {
//                 let value = cx.use_state(|| "B".to_owned());
//                 label(value.get()).into()
//             })
//             .key("b");

//             if reversed_for_app.get() {
//                 row().child(second).child(first).into()
//             } else {
//                 row().child(first).child(second).into()
//             }
//         });

//         let mut backend = MockRenderBackend::default();
//         app.resize(Size::new(240.0, 120.0));
//         app.render(&mut backend).unwrap();

//         let root = app.arena().root();
//         let row_id = app.arena().children(root)[0];
//         let first_before = app.arena().children(row_id)[0];
//         let second_before = app.arena().children(row_id)[1];

//         reversed.set(true);
//         app.mark_needs_rebuild();
//         app.render(&mut backend).unwrap();

//         let first_after = app.arena().children(row_id)[0];
//         let second_after = app.arena().children(row_id)[1];
//         assert_eq!(first_after, second_before);
//         assert_eq!(second_after, first_before);
//         assert!(matches!(
//             &app.arena().node(first_after).unwrap().kind,
//             xui_interface::WidgetKind::Label { text, .. } if text == "B"
//         ));
//         assert!(matches!(
//             &app.arena().node(second_after).unwrap().kind,
//             xui_interface::WidgetKind::Label { text, .. } if text == "A"
//         ));
//     }

//     #[test]
//     fn component_child_participates_in_layout() {
//         let mut app = app(|_| {
//             column()
//                 .child(component(|_| {
//                     container().size(Size::new(50.0, 20.0)).into()
//                 }))
//                 .child(container().size(Size::new(30.0, 10.0)))
//                 .into()
//         });
//         let mut backend = MockRenderBackend::default();

//         app.resize(Size::new(200.0, 200.0));
//         app.render(&mut backend).unwrap();

//         let root = app.arena().root();
//         let column_id = app.arena().children(root)[0];
//         let first = app.arena().children(column_id)[0];
//         let second = app.arena().children(column_id)[1];

//         assert_eq!(app.arena().node(first).unwrap().layout.width, 50.0);
//         assert_eq!(app.arena().node(first).unwrap().layout.height, 20.0);
//         assert_eq!(app.arena().node(second).unwrap().layout.y, 20.0);
//     }

//     #[test]
//     fn component_render_update_refreshes_event_handler() {
//         let value = Rc::new(Cell::new(1));
//         let seen = Rc::new(Cell::new(0));
//         let value_for_app = value.clone();
//         let seen_for_app = seen.clone();

//         let mut app = app(move |_| {
//             let value_for_component = value_for_app.clone();
//             let seen_for_component = seen_for_app.clone();
//             component(move |_| {
//                 let captured = value_for_component.get();
//                 let seen_for_click = seen_for_component.clone();
//                 button("capture")
//                     .on_click(move || seen_for_click.set(captured))
//                     .into()
//             })
//             .into()
//         });
//         let mut backend = MockRenderBackend::default();

//         app.resize(Size::new(200.0, 120.0));
//         app.render(&mut backend).unwrap();

//         value.set(2);
//         app.mark_needs_rebuild();
//         app.render(&mut backend).unwrap();

//         let root = app.arena().root();
//         let button_id = app.arena().children(root)[0];
//         click(&mut app, button_id);

//         assert_eq!(seen.get(), 2);
//     }

//     #[test]
//     fn same_key_different_component_type_replaces_subtree() {
//         fn first(_: &mut crate::state::HookContext<'_>) -> Element {
//             label("first").into()
//         }

//         fn second(_: &mut crate::state::HookContext<'_>) -> Element {
//             label("second").into()
//         }

//         let use_second = Rc::new(Cell::new(false));
//         let use_second_for_app = use_second.clone();
//         let mut app = app(move |_| {
//             if use_second_for_app.get() {
//                 component(second).key("same").into()
//             } else {
//                 component(first).key("same").into()
//             }
//         });
//         let mut backend = MockRenderBackend::default();

//         app.resize(Size::new(200.0, 120.0));
//         app.render(&mut backend).unwrap();

//         let root = app.arena().root();
//         let first_node = app.arena().children(root)[0];

//         use_second.set(true);
//         app.mark_needs_rebuild();
//         app.render(&mut backend).unwrap();

//         let second_node = app.arena().children(root)[0];
//         assert_ne!(first_node, second_node);
//         assert!(!app.arena().contains(first_node));
//         assert!(matches!(
//             &app.arena().node(second_node).unwrap().kind,
//             xui_interface::WidgetKind::Label { text, .. } if text == "second"
//         ));
//     }

//     #[test]
//     fn deleting_component_removes_rendered_widget_subtree() {
//         let show = Rc::new(Cell::new(true));
//         let show_for_app = show.clone();
//         let mut app = app(move |_| {
//             let mut root = column();
//             if show_for_app.get() {
//                 root = root.child(component(|_| {
//                     container()
//                         .key("box")
//                         .child(label("child").key("child"))
//                         .into()
//                 }));
//             }
//             root.into()
//         });
//         let mut backend = MockRenderBackend::default();

//         app.resize(Size::new(200.0, 120.0));
//         app.render(&mut backend).unwrap();

//         let root = app.arena().root();
//         let column_id = app.arena().children(root)[0];
//         let box_id = app.arena().children(column_id)[0];
//         let child_id = app.arena().children(box_id)[0];

//         show.set(false);
//         app.mark_needs_rebuild();
//         app.render(&mut backend).unwrap();

//         assert!(app.arena().children(column_id).is_empty());
//         assert!(!app.arena().contains(box_id));
//         assert!(!app.arena().contains(child_id));
//     }
// }

// // #[cfg(test)]
// // mod tests {
// //     use std::cell::Cell;
// //     use std::rc::Rc;

// //     use taffy::prelude as tf;

// //     use super::*;
// //     use crate::layout::MockTextMeasurer;

// //     #[test]
// //     fn arena_removes_subtrees_and_invalidates_old_node_ids() {
// //         let mut arena = UiArena::new();
// //         let parent = arena.insert(
// //             arena.root(),
// //             widgets::WidgetKind::Container {
// //                 background: Color::TRANSPARENT,
// //             },
// //             tf::Style::default(),
// //         );
// //         let child = arena.insert(
// //             parent,
// //             widgets::WidgetKind::Label {
// //                 text: "child".to_owned(),
// //                 color: Color::BLACK,
// //                 font_size: 14.0,
// //             },
// //             tf::Style::default(),
// //         );

// //         assert_eq!(arena.children(parent), &[child]);

// //         arena.remove_subtree(parent);

// //         assert!(!arena.contains(parent));
// //         assert!(!arena.contains(child));

// //         let replacement = arena.insert(
// //             arena.root(),
// //             widgets::WidgetKind::Label {
// //                 text: "replacement".to_owned(),
// //                 color: Color::BLACK,
// //                 font_size: 14.0,
// //             },
// //             tf::Style::default(),
// //         );

// //         assert_ne!(child, replacement);
// //     }

// //     #[test]
// //     fn taffy_lays_out_column_children() {
// //         let mut app = app(|_| {
// //             column()
// //                 .child(container().size(Size::new(50.0, 20.0)))
// //                 .child(container().size(Size::new(30.0, 10.0)))
// //                 .into()
// //         });
// //         let mut backend = MockRenderBackend::default();

// //         app.resize(Size::new(200.0, 200.0));
// //         app.render(&mut backend).unwrap();

// //         let root = app.arena().root();
// //         let column_id = app.arena().children(root)[0];
// //         let first = app.arena().children(column_id)[0];
// //         let second = app.arena().children(column_id)[1];

// //         assert_eq!(app.arena().node(first).unwrap().layout.width, 50.0);
// //         assert_eq!(app.arena().node(first).unwrap().layout.height, 20.0);
// //         assert_eq!(app.arena().node(second).unwrap().layout.y, 20.0);
// //     }

// //     #[test]
// //     fn hit_test_returns_deepest_visible_node() {
// //         let mut app = app(|_| {
// //             column()
// //                 .child(container().size(Size::new(80.0, 30.0)))
// //                 .into()
// //         });
// //         let mut backend = MockRenderBackend::default();

// //         app.resize(Size::new(100.0, 100.0));
// //         app.render(&mut backend).unwrap();

// //         let root = app.arena().root();
// //         let column_id = app.arena().children(root)[0];
// //         let child = app.arena().children(column_id)[0];

// //         assert_eq!(app.arena().hit_test(Point::new(10.0, 10.0)), Some(child));
// //     }

// //     #[test]
// //     fn button_click_updates_hook_state_and_repaints_locally() {
// //         let mut app = app(|cx| {
// //             let count = cx.use_state(|| 0);
// //             let count_for_click = count.clone();

// //             column()
// //                 .child(label(format!("count: {}", count.get())))
// //                 .child(button("Increment").on_click(move || {
// //                     count_for_click.set(count_for_click.get() + 1);
// //                 }))
// //                 .into()
// //         });
// //         let mut backend = MockRenderBackend::default();

// //         app.resize(Size::new(240.0, 120.0));
// //         app.render(&mut backend).unwrap();

// //         let root = app.arena().root();
// //         let column_id = app.arena().children(root)[0];
// //         let button_id = app.arena().children(column_id)[1];
// //         let button_rect = app.arena().node(button_id).unwrap().layout;
// //         let click = Point::new(button_rect.x + 2.0, button_rect.y + 2.0);

// //         app.dispatch_event(Event::PointerDown {
// //             position: click,
// //             button: PointerButton::Primary,
// //         });
// //         app.dispatch_event(Event::PointerUp {
// //             position: click,
// //             button: PointerButton::Primary,
// //         });
// //         app.render(&mut backend).unwrap();

// //         let column_id = app.arena().children(root)[0];
// //         let label_id = app.arena().children(column_id)[0];
// //         let label_node = app.arena().node(label_id).unwrap();

// //         assert!(matches!(
// //             &label_node.kind,
// //             widgets::WidgetKind::Label { text, .. } if text == "count: 1"
// //         ));
// //         assert!(!backend.last_damage.is_empty());
// //         assert!(backend.last_commands.iter().any(|command| {
// //             matches!(command, PaintCommand::Text { text, .. } if text == "count: 1")
// //         }));
// //     }

// //     #[test]
// //     fn custom_event_handler_can_consume_events() {
// //         let consumed = Rc::new(Cell::new(false));
// //         let consumed_for_handler = consumed.clone();
// //         let mut arena = UiArena::new();
// //         let id = arena.insert(
// //             arena.root(),
// //             widgets::WidgetKind::Container {
// //                 background: Color::TRANSPARENT,
// //             },
// //             tf::Style {
// //                 size: tf::Size {
// //                     width: tf::Dimension::length(20.0),
// //                     height: tf::Dimension::length(20.0),
// //                 },
// //                 ..Default::default()
// //             },
// //         );
// //         arena.node_mut(id).unwrap().on_event = Some(Box::new(move |_, _| {
// //             consumed_for_handler.set(true);
// //             EventResult::Consumed
// //         }));
// //         arena.compute_layout(Size::new(100.0, 100.0));

// //         let result = arena.dispatch_event(&Event::PointerDown {
// //             position: Point::new(1.0, 1.0),
// //             button: PointerButton::Primary,
// //         });

// //         assert_eq!(result, EventResult::Consumed);
// //         assert!(consumed.get());
// //     }

// //     #[test]
// //     fn child_dirty_marks_parent_subtree_dirty() {
// //         let mut arena = UiArena::new();
// //         let child = arena.insert(
// //             arena.root(),
// //             widgets::WidgetKind::Container {
// //                 background: Color::TRANSPARENT,
// //             },
// //             tf::Style::default(),
// //         );
// //         let root = arena.root();
// //         arena.clear_dirty(root);
// //         arena.clear_dirty(child);

// //         arena.mark_dirty(child, DirtyFlags::PAINT);

// //         assert!(arena.node(child).unwrap().dirty.contains(DirtyFlags::PAINT));
// //         assert!(
// //             arena
// //                 .node(root)
// //                 .unwrap()
// //                 .subtree_dirty
// //                 .contains(DirtyFlags::PAINT)
// //         );
// //     }

// //     #[test]
// //     fn clean_subtree_update_bails_out() {
// //         let mut app = app(|_| label("stable").into());
// //         let mut backend = MockRenderBackend::default();
// //         app.resize(Size::new(100.0, 100.0));
// //         app.render(&mut backend).unwrap();

// //         app.arena_mut().update_visits = 0;
// //         app.render(&mut backend).unwrap();

// //         assert_eq!(app.arena().update_visits, 0);
// //     }

// //     #[test]
// //     fn same_type_and_key_reuses_node() {
// //         let mut arena = UiArena::new();
// //         let mut measurer = MockTextMeasurer::default();
// //         let root = arena.root();

// //         arena.diff_children(root, vec![label("one").key("a").into()], &mut measurer);
// //         let first_id = arena.children(root)[0];

// //         arena.diff_children(root, vec![label("two").key("a").into()], &mut measurer);
// //         let second_id = arena.children(root)[0];

// //         assert_eq!(first_id, second_id);
// //         assert!(matches!(
// //             &arena.node(second_id).unwrap().kind,
// //             widgets::WidgetKind::Label { text, .. } if text == "two"
// //         ));
// //     }

// //     #[test]
// //     fn different_type_replaces_node_even_with_same_key() {
// //         let mut arena = UiArena::new();
// //         let mut measurer = MockTextMeasurer::default();
// //         let root = arena.root();

// //         arena.diff_children(root, vec![label("one").key("a").into()], &mut measurer);
// //         let label_id = arena.children(root)[0];

// //         arena.diff_children(root, vec![button("one").key("a").into()], &mut measurer);
// //         let button_id = arena.children(root)[0];

// //         assert_ne!(label_id, button_id);
// //         assert!(!arena.contains(label_id));
// //         assert_eq!(arena.node(button_id).unwrap().node_type, NodeType::Button);
// //     }

// //     #[test]
// //     fn color_change_repaints_without_layout() {
// //         let color = Rc::new(Cell::new(Color::BLACK));
// //         let color_for_app = color.clone();
// //         let mut app = app(move |_| label("stable").color(color_for_app.get()).into());
// //         let mut backend = MockRenderBackend::default();
// //         app.resize(Size::new(100.0, 100.0));
// //         app.render(&mut backend).unwrap();

// //         app.arena_mut().layout_passes = 0;
// //         app.arena_mut().repaint_passes = 0;
// //         color.set(Color::BLUE_500);
// //         app.mark_needs_rebuild();
// //         app.render(&mut backend).unwrap();

// //         assert_eq!(app.arena().layout_passes, 0);
// //         assert!(app.arena().repaint_passes > 0);
// //     }

// //     #[test]
// //     fn width_height_change_triggers_layout() {
// //         let width = Rc::new(Cell::new(40.0));
// //         let width_for_app = width.clone();
// //         let mut app = app(move |_| {
// //             container()
// //                 .size(Size::new(width_for_app.get(), 20.0))
// //                 .key("box")
// //                 .into()
// //         });
// //         let mut backend = MockRenderBackend::default();
// //         app.resize(Size::new(100.0, 100.0));
// //         app.render(&mut backend).unwrap();

// //         app.arena_mut().layout_passes = 0;
// //         width.set(80.0);
// //         app.mark_needs_rebuild();
// //         app.render(&mut backend).unwrap();

// //         assert!(app.arena().layout_passes > 0);
// //     }

// //     #[test]
// //     fn deleting_node_releases_old_subtree() {
// //         let mut arena = UiArena::new();
// //         let measurer = MockTextMeasurer::default();
// //         let root = arena.root();

// //         arena.diff_children(
// //             root,
// //             vec![
// //                 container()
// //                     .key("parent")
// //                     .child(label("child").key("child"))
// //                     .into(),
// //             ],
// //             &mut measurer,
// //         );
// //         let parent = arena.children(root)[0];
// //         let child = arena.children(parent)[0];

// //         arena.diff_children(root, Vec::new(), &mut measurer);

// //         assert!(!arena.contains(parent));
// //         assert!(!arena.contains(child));
// //     }

// //     #[test]
// //     fn keyed_insert_preserves_existing_node_state() {
// //         let mut arena = UiArena::new();
// //         let measurer = MockTextMeasurer::default();
// //         let root = arena.root();

// //         arena.diff_children(
// //             root,
// //             vec![button("A").key("a").into(), button("B").key("b").into()],
// //             &mut measurer,
// //         );
// //         let a = arena.children(root)[0];
// //         if let widgets::WidgetKind::Button { pressed, .. } = &mut arena.node_mut(a).unwrap().kind {
// //             *pressed = true;
// //         }

// //         arena.diff_children(
// //             root,
// //             vec![
// //                 button("X").key("x").into(),
// //                 button("B").key("b").into(),
// //                 button("A").key("a").into(),
// //             ],
// //             &mut measurer,
// //         );
// //         let moved_a = arena.children(root)[2];

// //         assert_eq!(a, moved_a);
// //         assert!(matches!(
// //             &arena.node(moved_a).unwrap().kind,
// //             widgets::WidgetKind::Button { pressed: true, .. }
// //         ));
// //     }

// //     #[test]
// //     fn gui_runtime_processes_events_and_renders_frame() {
// //         let clicked = Rc::new(Cell::new(false));
// //         let clicked_for_app = clicked.clone();
// //         let mut runtime = GuiRuntime::new(
// //             app(move |_| {
// //                 button("Click")
// //                     .on_click({
// //                         let clicked_for_click = clicked_for_app.clone();
// //                         move || clicked_for_click.set(true)
// //                     })
// //                     .into()
// //             }),
// //             MockRenderBackend::default(),
// //         );
// //         runtime.app_mut().resize(Size::new(100.0, 100.0));
// //         runtime.frame().unwrap();

// //         let root = runtime.app().arena().root();
// //         let button_id = runtime.app().arena().children(root)[0];
// //         let button_rect = runtime.app().arena().node(button_id).unwrap().layout;
// //         let point = Point::new(button_rect.x + 2.0, button_rect.y + 2.0);
// //         let mut events = QueueEventSource::new([
// //             RuntimeEvent::Input(Event::PointerDown {
// //                 position: point,
// //                 button: PointerButton::Primary,
// //             }),
// //             RuntimeEvent::Input(Event::PointerUp {
// //                 position: point,
// //                 button: PointerButton::Primary,
// //             }),
// //         ]);

// //         let report = runtime.tick(&mut events).unwrap();

// //         assert!(clicked.get());
// //         assert!(report.rendered);
// //         assert!(runtime.backend().frames >= 2);
// //     }

// //     #[test]
// //     fn node_paint_uses_dynamic_widget_trait() {
// //         let mut app = app(|_| label("dyn").into());
// //         let mut backend = MockRenderBackend::default();

// //         app.resize(Size::new(100.0, 100.0));
// //         app.render(&mut backend).unwrap();

// //         assert!(backend.last_commands.iter().any(|command| {
// //             matches!(command, PaintCommand::Text { text, .. } if text == "dyn")
// //         }));
// //         let root = app.arena().root();
// //         let label_id = app.arena().children(root)[0];
// //         assert_eq!(
// //             app.arena().node(label_id).unwrap().widget.node_type(),
// //             NodeType::Label
// //         );
// //     }
// // }
