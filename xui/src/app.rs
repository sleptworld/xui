use crate::ElementDesc;
use crate::component::ComponentRuntime;
use crate::core::Size;
use crate::event_system::translator::EventTranslator;
use crate::lanes::{event_lane, with_update_lane};
use crate::render::RenderBackend;
use crate::state::{AsyncDispatcher, AsyncMessage, HookContext, Scheduler};
use crate::style::Theme;
use crate::text::TextHost;
use crate::ui_runtime::{RenderFrameError, UiRuntime};
use std::future::Future;
use std::sync::mpsc;
use std::time::Duration;
use tokio::runtime::{
    Builder as TokioRuntimeBuilder, Handle as TokioHandle, Runtime as TokioRuntime,
};
use tokio::task::JoinHandle;
use xui_interface::TextBackend;
use xui_interface::events::{EventResult, RawEvent};

pub type ComponentFn = for<'a, 'b> fn(&'a mut HookContext<'b>) -> ElementDesc;

#[derive(Debug)]
pub enum AppRenderError<E> {
    Frame(RenderFrameError),
    Backend(E),
}

impl<E: std::fmt::Display> std::fmt::Display for AppRenderError<E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Frame(error) => write!(formatter, "render frame compilation failed: {error}"),
            Self::Backend(error) => write!(formatter, "render backend failed: {error}"),
        }
    }
}

impl<E> std::error::Error for AppRenderError<E> where E: std::error::Error + 'static {}

thread_local! {
    pub(crate) static TOKIO_RUNTIME: TokioRuntime = App::create_tokio_runtime();
}

pub struct App {
    ui_runtime: UiRuntime,
    components: ComponentRuntime,
    event_translator: EventTranslator,
    size: Size<f32>,
    tokio_runtime: TokioRuntime,
    async_dispatcher: AsyncDispatcher,
    async_receiver: mpsc::Receiver<AsyncMessage>,
}

impl App {
    pub(crate) fn resolve_local_shortcut(
        &self,
        event: &xui_interface::RawKeyboard,
    ) -> Option<(xui_interface::NodeId, xui_interface::ShortcutBinding)> {
        self.ui_runtime.resolve_local_shortcut(event)
    }

    pub(crate) fn command_root(&self) -> xui_interface::NodeId {
        self.ui_runtime
            .children(self.ui_runtime.root())
            .find(|child| *child != self.ui_runtime.root_overlayer())
            .unwrap_or(self.ui_runtime.root())
    }

    pub(crate) fn dispatch_command<T: TextBackend>(
        &mut self,
        target: xui_interface::NodeId,
        binding: xui_interface::ShortcutBinding,
        raw: &xui_interface::RawKeyboard,
        text: &mut TextHost<T>,
    ) -> EventResult {
        let event =
            self.event_translator
                .command_event(target, binding.command, binding.shortcut, raw);
        let result =
            crate::event_system::dispatcher::dispatch_semantic(&mut self.ui_runtime, text, event)
                .result;
        if self.scheduler().is_dirty() {
            self.components.mark_root_dirty();
        }
        result
    }
    pub fn new(root_component: ComponentFn) -> Self {
        let ui_runtime = UiRuntime::new();
        let scheduler = Scheduler::default();
        let tokio_runtime = Self::create_tokio_runtime();
        let (async_sender, async_receiver) = mpsc::channel();
        let async_dispatcher = AsyncDispatcher::new(async_sender);
        let components = ComponentRuntime::new_with_async(
            ui_runtime.root(),
            scheduler,
            async_dispatcher.clone(),
            Some(tokio_runtime.handle().clone()),
            root_component,
        );
        Self {
            ui_runtime,
            components,
            event_translator: EventTranslator::default(),
            size: Size::<f32>::ZERO,
            tokio_runtime,
            async_dispatcher,
            async_receiver,
        }
    }

    fn create_tokio_runtime() -> TokioRuntime {
        TokioRuntimeBuilder::new_multi_thread()
            .thread_name("xui-app-runtime")
            .enable_all()
            .build()
            .expect("failed to create xui app tokio runtime")
    }

    pub fn ui_runtime(&self) -> &UiRuntime {
        &self.ui_runtime
    }

    pub fn ui_runtime_mut(&mut self) -> &mut UiRuntime {
        &mut self.ui_runtime
    }

    pub fn theme(&self) -> &Theme {
        self.ui_runtime.theme()
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.ui_runtime.set_theme(theme);
    }

    pub fn set_rebuild_budget(&mut self, budget: Duration) {
        self.components.set_budget(budget);
    }

    pub fn tokio_handle(&self) -> TokioHandle {
        self.tokio_runtime.handle().clone()
    }

    pub fn spawn_background<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.tokio_runtime.spawn(future)
    }

    pub fn set_async_wake_callback(&self, wake: impl Fn() + Send + Sync + 'static) {
        self.async_dispatcher.set_wake_callback(wake);
    }

    pub fn drain_async_messages(&mut self) -> bool {
        let mut changed = false;
        while let Ok(message) = self.async_receiver.try_recv() {
            changed |= self.components.scheduler().enqueue_async_message(message);
        }

        if changed {
            self.components.mark_root_dirty();
        }
        changed
    }

    pub fn resize(&mut self, size: Size<f32>) {
        if self.size != size {
            self.size = size;
            self.ui_runtime
                .mark_subtree_layout_dirty(self.ui_runtime.root());
        }
    }

    fn scheduler(&self) -> &Scheduler {
        self.components.scheduler()
    }

    pub fn dispatch_event<T: TextBackend>(
        &mut self,
        event: RawEvent,
        text: &mut TextHost<T>,
    ) -> EventResult {
        self.rebuild_sync_if_needed();
        self.ui_runtime.update_tree(self.size, text);
        let lane = event_lane(&event);
        let result = with_update_lane(lane, || {
            self.ui_runtime
                .dispatch_event(text, &mut self.event_translator, event)
        });
        if self.scheduler().is_dirty() {
            self.components.mark_root_dirty();
        }
        result
    }

    #[inline(always)]
    pub fn tick_style_animations(&mut self, delta: Duration) -> bool {
        self.ui_runtime.tick_style_animations(delta)
    }

    #[inline(always)]
    pub fn has_running_style_animations(&self) -> bool {
        self.ui_runtime.has_running_style_animations()
    }

    pub fn render<B: RenderBackend<TextHost<T>>, T: TextBackend>(
        &mut self,
        backend: &mut B,
        text: &mut TextHost<T>,
    ) -> Result<(), AppRenderError<B::Error>> {
        self.drain_async_messages();

        if self.rebuild_slice_if_needed() {
            self.flush_node_lifecycle(backend, text);
        }

        self.ui_runtime.update_tree(self.size, text);

        let Some(frame) = self
            .ui_runtime
            .build_render_frame()
            .map_err(AppRenderError::Frame)?
        else {
            return Ok(());
        };

        backend
            .begin_frame(self.size)
            .map_err(AppRenderError::Backend)?;
        backend
            .submit(&frame.built, text)
            .map_err(AppRenderError::Backend)?;
        backend.end_frame().map_err(AppRenderError::Backend)?;
        if backend.did_present() {
            self.ui_runtime.finish_render_frame(&frame);
        }
        Ok(())
    }

    pub fn is_dirty(&self) -> bool {
        self.components.is_dirty() || self.ui_runtime.is_dirty()
    }

    #[inline]
    pub fn mark_needs_rebuild(&mut self) {
        self.components.mark_root_dirty();
    }

    #[inline]
    fn rebuild_sync_if_needed(&mut self) {
        self.components.rebuild_sync_if_needed(&mut self.ui_runtime);
    }

    #[inline]
    fn rebuild_slice_if_needed(&mut self) -> bool {
        self.components
            .rebuild_slice_if_needed(&mut self.ui_runtime)
    }

    fn flush_node_lifecycle<B: RenderBackend<TextHost<T>>, T: TextBackend>(
        &mut self,
        backend: &mut B,
        text: &mut TextHost<T>,
    ) {
        for event in self.ui_runtime.drain_node_lifecycle_events() {
            text.handle_node_lifecycle(&event);
            backend.handle_node_lifecycle(&event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lanes::{DEFAULT_LANE, NO_LANES};
    use crate::prelude::{CanvasController, canvas, container};
    use crate::render::{BuiltDraw, BuiltFrame, BuiltItem, MockRenderBackend, RenderBackend};
    use crate::state::State;
    use crate::text::{TextHost, testing::ZeroTextBackend};
    use core::convert::Infallible;
    use std::cell::RefCell;
    use std::time::Instant;
    use xui_interface::events::{
        Modifiers, PointerButton, PointerButtons, PointerKind, RawPointerButton, XuiPointerId,
    };
    use xui_interface::{
        Affine, Color, ComputedColorStyle, PathBuilder, PathFill, Point, Style, VectorScene,
        VectorSceneBuilder,
    };

    #[derive(Default)]
    struct NoPresentBackend {
        inner: MockRenderBackend,
    }

    impl<T> RenderBackend<T> for NoPresentBackend {
        type Error = Infallible;

        fn begin_frame(&mut self, size: Size<f32>) -> Result<(), Self::Error> {
            self.inner.frame_size = Some(size);
            Ok(())
        }

        fn submit(&mut self, frame: &BuiltFrame, _text: &mut T) -> Result<(), Self::Error> {
            self.inner.last_frame = Some(frame.clone());
            Ok(())
        }

        fn end_frame(&mut self) -> Result<(), Self::Error> {
            self.inner.frames += 1;
            Ok(())
        }

        fn did_present(&self) -> bool {
            false
        }
    }

    thread_local! {
        static STATE_SLOT: RefCell<Option<State<bool>>> = const { RefCell::new(None) };
        static CANVAS_SLOT: RefCell<Option<CanvasController>> = const { RefCell::new(None) };
    }

    fn canvas_scene(color: Color) -> VectorScene {
        let mut path = PathBuilder::new();
        path.move_to(Point::new(0.0, 0.0))
            .line_to(Point::new(40.0, 0.0))
            .line_to(Point::new(40.0, 20.0))
            .line_to(Point::new(0.0, 20.0))
            .close();
        let mut scene = VectorSceneBuilder::new();
        scene.fill_path(path.build(), Affine::IDENTITY, PathFill::new(color));
        scene.build()
    }

    fn clickable_canvas_root(cx: &mut HookContext<'_>) -> ElementDesc {
        let highlighted = cx.use_state(|| false);
        let controller = cx.use_ref(|| CanvasController::with_scene(canvas_scene(Color::BLACK)));
        let canvas_handle = controller.get().clone();
        let click_handle = canvas_handle.clone();
        CANVAS_SLOT.with(|slot| {
            *slot.borrow_mut() = Some(canvas_handle.clone());
        });
        canvas(canvas_handle)
            .style(Style::new().width(40.0).height(20.0))
            .on_click(move |_, _| {
                let next = !*highlighted.get();
                click_handle.set_scene(canvas_scene(if next {
                    Color::WHITE
                } else {
                    Color::BLACK
                }));
                highlighted.set(next);
                EventResult::Consumed
            })
            .into_element_desc()
    }

    fn static_root(_cx: &mut HookContext<'_>) -> ElementDesc {
        container()
            .style(
                Style::new()
                    .width(40.0)
                    .height(20.0)
                    .background(Color::BLACK),
            )
            .into_element_desc(Vec::new())
    }

    fn stateful_root(cx: &mut HookContext<'_>) -> ElementDesc {
        let state = cx.use_state(|| false);
        STATE_SLOT.with(|slot| {
            *slot.borrow_mut() = Some(state);
        });
        let color = if *state.get() {
            Color::WHITE
        } else {
            Color::BLACK
        };
        container()
            .style(Style::new().width(40.0).height(20.0).background(color))
            .into_element_desc(Vec::new())
    }
}
