use crate::ElementDesc;
use crate::component::ComponentRuntime;
use crate::core::{Rect, Size};
use crate::event::{Event, EventResult};
use crate::fiber::ComponentRegistry;
use crate::lanes::{event_lane, with_update_lane};
use crate::render::RenderBackend;
use crate::state::{HookContext, Scheduler};
use crate::style::Theme;
use crate::tree::UiArena;
use std::time::Duration;
use xui_interface::{DirtyFlags, TextMeasurer};

pub struct App {
    arena: UiArena,
    components: ComponentRuntime,
    size: Size<f32>,
}

impl App {
    pub fn new(
        root_component: impl for<'a> FnMut(&mut HookContext<'a>) -> ElementDesc + 'static,
    ) -> Self {
        Self::with_component_registry(|_| root_component)
    }

    pub fn with_component_registry<I, F>(init_components: I) -> Self
    where
        I: FnOnce(&mut ComponentRegistry) -> F,
        F: for<'a> FnMut(&mut HookContext<'a>) -> ElementDesc + 'static,
    {
        let arena = UiArena::new();
        let scheduler = Scheduler::default();
        let components = ComponentRuntime::new(arena.root(), scheduler, init_components);
        let app = Self {
            arena,
            components,
            size: Size::<f32>::ZERO,
        };
        // app.rebuild_if_needed(measure);
        app
    }

    pub fn arena(&self) -> &UiArena {
        &self.arena
    }

    pub fn arena_mut(&mut self) -> &mut UiArena {
        &mut self.arena
    }

    pub fn theme(&self) -> &Theme {
        self.arena.theme()
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.arena.set_theme(theme);
    }

    pub fn set_rebuild_budget(&mut self, budget: Duration) {
        self.components.set_budget(budget);
    }

    pub fn resize(&mut self, size: Size<f32>) {
        if self.size != size {
            self.size = size;
            self.arena.mark_subtree_layout_dirty(self.arena.root());
            self.arena
                .add_damage(Rect::new(0.0, 0.0, size.width, size.height));
        }
    }

    fn scheduler(&self) -> &Scheduler {
        self.components.scheduler()
    }

    pub fn dispatch_event<T: TextMeasurer>(&mut self, event: Event, m: &mut T) -> EventResult {
        self.rebuild_sync_if_needed();
        self.arena.update_tree(self.arena.root(), self.size, m);
        let lane = event_lane(&event);
        let result = with_update_lane(lane, || self.arena.dispatch_event(&event));
        if self.scheduler().is_dirty() {
            self.arena.mark_dirty(self.arena.root(), DirtyFlags::STATE);
        }
        result
    }

    pub fn render<B: RenderBackend<T>, T: TextMeasurer>(
        &mut self,
        backend: &mut B,
        m: &mut T,
    ) -> Result<(), B::Error> {
        if !self.rebuild_slice_if_needed() {
            return Ok(());
        }
        self.arena.update_tree(self.arena.root(), self.size, m);

        let (damage, commands) = self.arena.prepare_paint_commands();
        if damage.is_empty() {
            return Ok(());
        }

        backend.begin_frame(self.size)?;
        backend.paint(&commands, &damage, m)?;
        backend.end_frame()?;
        if backend.did_present() {
            self.arena.finish_paint();
        }
        Ok(())
    }

    pub fn is_dirty(&self) -> bool {
        self.components.is_dirty() || self.arena.is_dirty()
    }

    #[inline]
    pub fn mark_needs_rebuild(&mut self) {
        self.components.mark_root_dirty();
        self.arena.mark_dirty(self.arena.root(), DirtyFlags::STATE);
        self.arena
            .add_damage(Rect::new(0.0, 0.0, self.size.width, self.size.height));
    }

    #[inline]
    fn rebuild_sync_if_needed(&mut self) {
        self.components.rebuild_sync_if_needed(&mut self.arena);
    }

    #[inline]
    fn rebuild_slice_if_needed(&mut self) -> bool {
        self.components.rebuild_slice_if_needed(&mut self.arena)
    }
}

pub fn app(
    root_component: impl for<'a> FnMut(&mut HookContext<'a>) -> ElementDesc + 'static,
) -> App {
    App::new(root_component)
}

// #[cfg(test)]
// mod tests {
//     use std::cell::RefCell;
//     use std::rc::Rc;

//     use crate::lanes::{INPUT_CONTINUOUS_LANE, SYNC_LANE};
//     use crate::prelude::*;
//     use crate::state::State;

//     fn app_with_event_state() -> (App, Rc<RefCell<Option<State<i32>>>>) {
//         let state_slot = Rc::new(RefCell::new(None));
//         let state_slot_for_app = state_slot.clone();
//         let mut app = app(move |cx| {
//             let state = cx.use_state(|| 0);
//             *state_slot_for_app.borrow_mut() = Some(state);
//             container().size(Size::new(40.0, 40.0)).into()
//         });
//         let mut backend = MockRenderBackend::default();
//         app.resize(Size::new(100.0, 100.0));
//         app.render(&mut backend).unwrap();
//         (app, state_slot)
//     }

//     #[test]
//     fn discrete_event_updates_use_sync_lane() {
//         let (mut app, state_slot) = app_with_event_state();
//         let root = app.arena().root();
//         let child = app.arena().children(root)[0];
//         let state = state_slot.borrow().as_ref().unwrap().clone();

//         app.arena_mut()
//             .node_mut(child)
//             .unwrap()
//             .event_handlers
//             .on_event = Some(Box::new(move |_, _| {
//             state.update(|value| *value += 1);
//             EventResult::Consumed
//         }));

//         app.dispatch_event(Event::PointerDown {
//             position: Point::new(2.0, 2.0),
//             button: PointerButton::Primary,
//         });

//         assert_eq!(app.scheduler.pending_lanes(), SYNC_LANE);
//         assert!(app.is_dirty());
//     }

//     #[test]
//     fn continuous_event_updates_use_continuous_input_lane() {
//         let (mut app, state_slot) = app_with_event_state();
//         let root = app.arena().root();
//         let child = app.arena().children(root)[0];
//         let state = state_slot.borrow().as_ref().unwrap().clone();

//         app.arena_mut()
//             .node_mut(child)
//             .unwrap()
//             .event_handlers
//             .on_event = Some(Box::new(move |_, _| {
//             state.update(|value| *value += 1);
//             EventResult::Consumed
//         }));

//         app.dispatch_event(Event::PointerMove {
//             position: Point::new(2.0, 2.0),
//         });

//         assert_eq!(app.scheduler.pending_lanes(), INPUT_CONTINUOUS_LANE);
//         assert!(app.is_dirty());
//     }

//     #[test]
//     fn runtime_processes_input_and_renders_scheduled_update() {
//         let (app, state_slot) = app_with_event_state();
//         let mut runtime = GuiRuntime::new(app, MockRenderBackend::default());
//         let root = runtime.app().arena().root();
//         let child = runtime.app().arena().children(root)[0];
//         let state = state_slot.borrow().as_ref().unwrap().clone();
//         let handler = Box::new(move |_: &Event, _: &mut EventContext<'_>| {
//             state.update(|value| *value += 1);
//             EventResult::Consumed
//         });

//         runtime
//             .app_mut()
//             .arena_mut()
//             .node_mut(child)
//             .unwrap()
//             .event_handlers
//             .on_event = Some(handler);

//         let mut events = QueueEventSource::new([RuntimeEvent::Input(Event::PointerDown {
//             position: Point::new(2.0, 2.0),
//             button: PointerButton::Primary,
//         })]);
//         let report = runtime.tick(&mut events).unwrap();

//         assert_eq!(report.event_results, vec![EventResult::Consumed]);
//         assert!(report.rendered);
//         assert_eq!(state_slot.borrow().as_ref().unwrap().get(), 1);
//     }
// }
