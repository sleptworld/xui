use crate::component::ComponentRuntime;
use crate::core::{Rect, Size};
use crate::event::{Event, EventResult};
use crate::font::TextI;
use crate::render::RenderBackend;
use crate::state::{HookContext, Scheduler};
use crate::tree::UiArena;
use crate::widgets::Element;
use xui_interface::DirtyFlags;

pub struct App {
    arena: UiArena,
    components: ComponentRuntime,
    scheduler: Scheduler,
    size: Size,
    texti: TextI,
}

impl App {
    pub fn new(
        root_component: impl for<'a> FnMut(&mut HookContext<'a>) -> Element + 'static,
    ) -> Self {
        let arena = UiArena::new();
        let scheduler = Scheduler::default();
        let components = ComponentRuntime::new(arena.root(), scheduler.clone(), root_component);
        let mut app = Self {
            arena,
            components,
            scheduler,
            size: Size::ZERO,
            texti: TextI::new(),
        };
        app.rebuild_if_needed();
        app
    }

    pub fn arena(&self) -> &UiArena {
        &self.arena
    }

    pub fn arena_mut(&mut self) -> &mut UiArena {
        &mut self.arena
    }

    pub fn resize(&mut self, size: Size) {
        if self.size != size {
            self.size = size;
            self.arena.mark_subtree_layout_dirty(self.arena.root());
            self.arena
                .add_damage(Rect::new(0.0, 0.0, size.width, size.height));
        }
    }

    pub fn dispatch_event(&mut self, event: Event) -> EventResult {
        self.rebuild_if_needed();
        self.arena.update_tree(self.arena.root(), self.size);
        let result = self.arena.dispatch_event(&event);
        if self.scheduler.is_dirty() {
            self.arena.mark_dirty(self.arena.root(), DirtyFlags::STATE);
        }
        result
    }

    pub fn render<B: RenderBackend>(&mut self, backend: &mut B) -> Result<(), B::Error> {
        self.rebuild_if_needed();
        self.arena.update_tree(self.arena.root(), self.size);

        let (damage, commands) = self.arena.prepare_paint_commands();
        if damage.is_empty() {
            return Ok(());
        }

        backend.begin_frame(self.size)?;
        backend.paint(&commands, &damage)?;
        backend.end_frame()?;
        if backend.did_present() {
            self.arena.finish_paint();
        }
        Ok(())
    }

    pub fn is_dirty(&self) -> bool {
        self.components.is_dirty() || self.arena.is_dirty()
    }

    pub fn mark_needs_rebuild(&mut self) {
        self.components.mark_root_dirty();
        self.arena.mark_dirty(self.arena.root(), DirtyFlags::STATE);
    }

    fn rebuild_if_needed(&mut self) {
        self.components
            .rebuild_if_needed(&mut self.arena, &mut self.texti);
    }
}

pub fn app(root_component: impl for<'a> FnMut(&mut HookContext<'a>) -> Element + 'static) -> App {
    App::new(root_component)
}
