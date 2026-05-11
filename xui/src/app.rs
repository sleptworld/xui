use std::cell::Cell;
use std::rc::Rc;

use crate::core::Size;
use crate::event::{Event, EventResult};
use crate::layout::MockTextMeasurer;
use crate::render::RenderBackend;
use crate::state::HookContext;
use crate::tree::UiArena;
use crate::widgets::Element;
use xui_interface::{DirtyFlags, TextMeasurer};

pub struct App {
    arena: UiArena,
    root_component: Box<dyn FnMut(&mut HookContext<'_>) -> Element>,
    dirty_signal: Rc<Cell<bool>>,
    size: Size,
    text_measurer: Box<dyn TextMeasurer>,
}

impl App {
    pub fn new(root_component: impl FnMut(&mut HookContext<'_>) -> Element + 'static) -> Self {
        let mut app = Self {
            arena: UiArena::new(),
            root_component: Box::new(root_component),
            dirty_signal: Rc::new(Cell::new(true)),
            size: Size::ZERO,
            text_measurer: Box::<MockTextMeasurer>::default(),
        };
        app.rebuild_if_needed();
        app
    }

    pub fn with_text_measurer(mut self, text_measurer: impl TextMeasurer + 'static) -> Self {
        self.text_measurer = Box::new(text_measurer);
        self.mark_needs_rebuild();
        self
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
        }
    }

    pub fn dispatch_event(&mut self, event: Event) -> EventResult {
        self.rebuild_if_needed();
        let result = self.arena.dispatch_event(&event);
        if self.dirty_signal.get() {
            self.arena.mark_dirty(self.arena.root(), DirtyFlags::STATE);
        }
        result
    }

    pub fn render<B: RenderBackend>(&mut self, backend: &mut B) -> Result<(), B::Error> {
        self.rebuild_if_needed();
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
        self.dirty_signal.get() || self.arena.is_dirty()
    }

    pub fn mark_needs_rebuild(&mut self) {
        self.dirty_signal.set(true);
        self.arena.mark_dirty(self.arena.root(), DirtyFlags::STATE);
    }

    fn rebuild_if_needed(&mut self) {
        let root = self.arena.root();
        let needs_rebuild = self.dirty_signal.replace(false)
            || self
                .arena
                .node(root)
                .is_some_and(|node| node.dirty.intersects(DirtyFlags::STATE | DirtyFlags::TREE));

        if !needs_rebuild {
            return;
        }

        let storage = self
            .arena
            .hooks
            .get_mut(root)
            .expect("root hook storage missing");
        let mut cx = HookContext::new(storage, self.dirty_signal.clone());
        let element = (self.root_component)(&mut cx);

        self.arena
            .diff_children(root, vec![element], self.text_measurer.as_ref());
        self.arena.mark_dirty(root, DirtyFlags::STATE);
    }
}

pub fn app(root_component: impl FnMut(&mut HookContext<'_>) -> Element + 'static) -> App {
    App::new(root_component)
}
