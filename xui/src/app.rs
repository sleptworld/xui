use crate::core::Size;
use crate::event::{Event, EventResult};
use crate::fiber::{FiberRuntime, WorkStatus};
use crate::font::TextI;
use crate::lanes::{SYNC_LANE, event_lane, includes_sync_lane, with_update_lane};
use crate::render::RenderBackend;
use crate::state::{HookContext, Scheduler};
use crate::tree::UiArena;
use crate::widgets::Element;

pub struct App {
    arena: UiArena,
    fiber: FiberRuntime,
    scheduler: Scheduler,
    size: Size,
    texti: TextI,
}

impl App {
    pub fn new(root_component: impl FnMut(&mut HookContext<'_>) -> Element + 'static) -> Self {
        let arena = UiArena::new();
        let scheduler = Scheduler::default();
        let fiber = FiberRuntime::new(arena.root(), scheduler.clone(), root_component);
        let mut app = Self {
            arena,
            fiber,
            scheduler,
            size: Size::ZERO,
            texti: TextI::new(),
        };
        app.flush_sync_rebuild();
        app
    }

    pub fn arena(&self) -> &UiArena {
        &self.arena
    }

    pub fn arena_mut(&mut self) -> &mut UiArena {
        &mut self.arena
    }

    pub fn fiber(&self) -> &FiberRuntime {
        &self.fiber
    }

    pub fn fiber_mut(&mut self) -> &mut FiberRuntime {
        &mut self.fiber
    }

    pub fn resize(&mut self, size: Size) {
        if self.size != size {
            self.size = size;
            self.arena.mark_subtree_layout_dirty(self.arena.root());
        }
    }

    pub fn dispatch_event(&mut self, event: Event) -> EventResult {
        self.flush_sync_rebuild();
        self.arena.update_tree(self.arena.root(), self.size);

        let lane = event_lane(&event);
        let result = with_update_lane(lane, || self.arena.dispatch_event(&event));

        if includes_sync_lane(self.scheduler.pending_lanes()) {
            self.flush_sync_rebuild();
        }

        result
    }

    pub fn render<B: RenderBackend>(&mut self, backend: &mut B) -> Result<(), B::Error> {
        let status = self
            .fiber
            .perform_budgeted_work(&mut self.arena, &mut self.texti);
        if matches!(status, WorkStatus::Yielded) {
            return Ok(());
        }

        if includes_sync_lane(self.scheduler.pending_lanes()) {
            self.flush_sync_rebuild();
        }

        self.arena.update_tree(self.arena.root(), self.size);

        let (damage, commands) = self.arena.collect_paint_commands();
        if damage.is_empty() {
            return Ok(());
        }

        backend.begin_frame(self.size)?;
        backend.paint(&commands, &damage)?;
        backend.end_frame()
    }

    pub fn is_dirty(&self) -> bool {
        self.fiber.is_dirty() || self.arena.is_dirty()
    }

    pub fn mark_needs_rebuild(&mut self) {
        with_update_lane(SYNC_LANE, || self.fiber.mark_root_dirty());
    }

    fn flush_sync_rebuild(&mut self) {
        self.fiber.flush_sync(&mut self.arena, &mut self.texti);
    }
}

pub fn app(root_component: impl FnMut(&mut HookContext<'_>) -> Element + 'static) -> App {
    App::new(root_component)
}
