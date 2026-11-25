use crate::clock::FrameTime;
use crate::core::Size;
use crate::event_system::dispatcher;
use crate::event_system::translator::EventTranslator;
use crate::platform::PlatformHandle;
use crate::render::RenderBackend;
use crate::scroll::AppliedScroll;
use crate::shortcut::ShortcutManager;
use crate::style::Theme;
use crate::system;
use crate::text::TextHost;
use crate::theme::ThemeHandle;
use crate::ui_runtime::{ExternalDrag, RenderFrameError, UiRuntime};
use std::future::Future;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::{
    Builder as TokioRuntimeBuilder, Handle as TokioHandle, Runtime as TokioRuntime,
};
use tokio::task::JoinHandle;
use xui_interface::TextBackend;
use xui_interface::events::SemanticEvent;
use xui_interface::events::{EventResult, RawEvent, ScrollPhase, ScrollSource};
use xui_interface::{Appearance, DropEffect, DropEvent, NodeId, Point, ThemeMode, Themes};

/// What an app starts with: typically mounts its root component,
/// `|app| { app.mount::<Gallery>(GalleryProps {}); }`.
pub type AppRoot = fn(&mut App);

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
    reactive: crate::reactive::Runtime,
    event_translator: EventTranslator,
    size: Size<f32>,
    tokio_runtime: TokioRuntime,
    /// Results of `resource!` futures, waiting for the main thread.
    finished: std::sync::mpsc::Receiver<(u64, Box<dyn std::any::Any + Send>)>,
    platform: PlatformHandle,
    theme: ThemeHandle,
    shortcuts: ShortcutManager,
    #[cfg(feature = "hotpatch")]
    hot: HotRoot,
}

/// What the app's root mounted, to mount again when a hot patch changes a root.
#[cfg(feature = "hotpatch")]
struct HotRoot {
    root: AppRoot,
    mounted: Vec<crate::reactive::InstanceId>,
}

impl App {
    /// Where the application leaves commands and requests for the host, and
    /// where the host's answers arrive. See [`crate::platform`].
    pub fn platform(&self) -> &PlatformHandle {
        &self.platform
    }

    /// Raises a command the platform itself originated -- a menu item, or one
    /// of the system's standard actions -- at the focused node, from where it
    /// bubbles like any command. With nothing focused it goes to the command
    /// root, as a global shortcut does.
    pub(crate) fn dispatch_platform_command<T: TextBackend>(
        &mut self,
        command: xui_interface::CommandId,
        timestamp: std::time::Instant,
        text: &mut TextHost<T>,
    ) -> EventResult {
        let target = self
            .ui_runtime
            .focus_manager()
            .focused()
            .unwrap_or_else(|| self.command_root());
        let event = self.event_translator.platform_command_event(
            target,
            command,
            timestamp,
            xui_interface::Modifiers::default(),
        );
        let result =
            crate::event_system::dispatcher::dispatch_semantic(&mut self.ui_runtime, text, event)
                .result;
        result
    }

    /// Delivers the host's answer to the request that asked for it. What the
    /// callback changed is picked up the way any event's changes are.
    pub(crate) fn deliver_platform_reply(
        &mut self,
        id: xui_interface::RequestId,
        reply: xui_interface::PlatformReply,
    ) {
        self.platform.deliver_reply(id, reply);
    }

    /// Delivers every wake due at `now` to the node that asked for it.
    ///
    /// Called by the frame loop before anything is stepped, so what a wake
    /// changes is painted in the frame that delivered it.
    pub(crate) fn deliver_due_wakes<T: TextBackend>(
        &mut self,
        now: std::time::Instant,
        text: &mut TextHost<T>,
    ) {
        let due = self.ui_runtime.take_due_wakes(now);
        if due.is_empty() {
            return;
        }
        for (node, deadline) in due {
            let event = self.event_translator.wake_event(node, deadline, now);
            crate::event_system::dispatcher::dispatch_semantic(&mut self.ui_runtime, text, event);
        }
    }

    /// When the next frame is owed to a wake, if one is. See
    /// [`crate::runtime::GuiRuntime::next_wake`].
    pub fn next_wake(&self) -> Option<std::time::Instant> {
        self.ui_runtime.next_wake()
    }

    /// Hands each answer to a request a node made to that node, as an event.
    ///
    /// Run after the replies of a settle are delivered, so the node handles them
    /// with its own context, like any other event. An answer for a node that has
    /// since gone is dropped, which `dispatch_semantic` does for a target that no
    /// longer exists.
    pub(crate) fn deliver_node_replies<T: TextBackend>(
        &mut self,
        timestamp: std::time::Instant,
        text: &mut TextHost<T>,
    ) -> EventResult {
        let mut result = EventResult::Ignored;
        for reply in self.platform.take_node_replies() {
            let event = self.event_translator.platform_reply_event(
                reply.node,
                reply.request,
                reply.reply,
                timestamp,
            );
            let report = crate::event_system::dispatcher::dispatch_semantic(
                &mut self.ui_runtime,
                text,
                event,
            );
            if report.result.is_consumed() {
                result = EventResult::Consumed;
            }
        }
        result
    }

    /// The app's own shortcuts: keys that mean a command wherever focus is.
    /// A key resolves here only after the focused widget and the shortcuts
    /// on its ancestors have passed, and its command goes to the command
    /// root, the root component's first node.
    pub fn shortcuts(&self) -> &ShortcutManager {
        &self.shortcuts
    }

    pub fn shortcuts_mut(&mut self) -> &mut ShortcutManager {
        &mut self.shortcuts
    }

    /// The command a key means and the node it goes to: a shortcut on the
    /// focus path first, then one of the app's own.
    pub(crate) fn resolve_shortcut(
        &self,
        event: &xui_interface::RawKeyboard,
    ) -> Option<(xui_interface::NodeId, xui_interface::ShortcutBinding)> {
        self.ui_runtime.resolve_local_shortcut(event).or_else(|| {
            self.shortcuts
                .resolve(event)
                .map(|binding| (self.command_root(), binding))
        })
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
        result
    }
    /// An app set up by `root`, which mounts what it shows.
    pub fn new(root: AppRoot) -> Self {
        let mut ui_runtime = UiRuntime::new();
        let platform = PlatformHandle::default();
        ui_runtime.set_platform(platform.clone());
        let tokio_runtime = Self::create_tokio_runtime();
        let mut reactive = crate::reactive::Runtime::new();
        let (sender, finished) = std::sync::mpsc::channel();
        let handle = tokio_runtime.handle().clone();
        reactive.tasks_mut().connect(
            Box::new(move |future| {
                handle.spawn(future);
            }),
            sender,
        );
        let appearance = platform.appearance();
        let theme = ThemeHandle::new(appearance.color_scheme);
        reactive.provide_context(theme.clone());
        reactive.provide_context(appearance);
        provide_system_defaults(&mut reactive);
        ui_runtime.set_theme(theme.current());
        let mut app = Self {
            ui_runtime,
            reactive,
            event_translator: EventTranslator::default(),
            size: Size::<f32>::ZERO,
            tokio_runtime,
            finished,
            platform,
            theme,
            shortcuts: ShortcutManager::default(),
            #[cfg(feature = "hotpatch")]
            hot: HotRoot {
                root,
                mounted: Vec::new(),
            },
        };
        #[cfg(feature = "hotpatch")]
        {
            crate::hot::register();
            subsecond::HotFn::current(root).call((&mut app,));
        }
        #[cfg(not(feature = "hotpatch"))]
        root(&mut app);
        app
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

    /// Mounts reactive component `C` at the root, and runs its first effects.
    pub fn mount<C: crate::reactive::Component>(
        &mut self,
        props: <C::Instance as crate::reactive::ComponentDef>::Props,
    ) -> crate::reactive::InstanceId {
        let root = self.ui_runtime.root();
        let id = self.reactive.mount::<C>(&mut self.ui_runtime, root, props);
        #[cfg(feature = "hotpatch")]
        self.hot.mounted.push(id);
        id
    }

    /// After a hot patch: mounts again what changed, keeping the rest; all
    /// of it when a root changed, or with `XUI_HOTPATCH=full`.
    #[cfg(feature = "hotpatch")]
    fn hot_remount(&mut self) {
        let start = std::time::Instant::now();
        let full = std::env::var("XUI_HOTPATCH").as_deref() == Ok("full");
        let outcome = if full {
            Err(())
        } else {
            self.reactive.hot_reload(&mut self.ui_runtime)
        };
        match outcome {
            Ok(report) => crate::hot::report_reload(
                report.changed,
                report.called,
                report.stale,
                report.remounted,
                report.kept,
                start.elapsed().as_micros() as u64,
                false,
            ),
            Err(()) => {
                let count = self.hot.mounted.len();
                for id in std::mem::take(&mut self.hot.mounted) {
                    self.reactive.unmount(&mut self.ui_runtime, id);
                }
                let root = self.hot.root;
                subsecond::HotFn::current(root).call((self,));
                crate::hot::report_reload(
                    count,
                    0,
                    0,
                    count,
                    0,
                    start.elapsed().as_micros() as u64,
                    true,
                );
            }
        }
        self.mark_needs_rebuild();
    }

    /// A signal the application owns rather than a component: state to hand
    /// a root as a `#[source]` prop, and to write with [`update`](Self::update).
    pub fn signal<T>(&mut self, value: T) -> crate::reactive::Signal<T> {
        self.reactive.signal(value)
    }

    /// The reactive runtime the app's components are mounted in.
    pub fn reactive(&self) -> &crate::reactive::Runtime {
        &self.reactive
    }

    /// Runs `f`, which writes signals as a handler would; the next frame
    /// (or event) flushes what it wrote.
    pub fn update(&mut self, f: impl FnOnce(&mut crate::reactive::Cx<'_>)) {
        f(&mut crate::reactive::Cx::handler(self.reactive.notifier()));
    }

    /// Unmounts a component [`mount`](Self::mount) mounted.
    pub fn unmount(&mut self, id: crate::reactive::InstanceId) {
        #[cfg(feature = "hotpatch")]
        self.hot.mounted.retain(|mounted| *mounted != id);
        self.reactive.unmount(&mut self.ui_runtime, id);
    }

    #[cfg(test)]
    pub(crate) fn ui_runtime_mut(&mut self) -> &mut UiRuntime {
        &mut self.ui_runtime
    }

    /// The theme in use.
    pub fn theme(&self) -> Rc<Theme> {
        self.theme.current()
    }

    /// The application's themes and which of them is in use: the handle
    /// components get through `context!()`.
    pub fn theme_handle(&self) -> &ThemeHandle {
        &self.theme
    }

    /// Sets the light and dark themes, at once.
    pub fn set_themes(&mut self, themes: Themes) {
        self.theme.set_themes(themes);
        self.apply_theme();
    }

    /// Sets which theme is in use, at once; [`ThemeMode::System`] (the
    /// default) follows the system's color scheme.
    pub fn set_theme_mode(&mut self, mode: ThemeMode) {
        self.theme.set_mode(mode);
        self.apply_theme();
    }

    /// Records the system's appearance, which readers of the `Appearance`
    /// context and [`ThemeMode::System`] follow.
    pub(crate) fn set_appearance(&mut self, appearance: Appearance) {
        self.platform.set_appearance(appearance);
        self.reactive.provide_context(appearance);
        self.apply_theme();
    }

    /// Restyles the tree against the theme the handle now selects, if that is
    /// another one.
    fn apply_theme(&mut self) {
        let system = self.platform.appearance().color_scheme;
        if let Some(theme) = self.theme.apply(system) {
            self.ui_runtime.set_theme(theme);
        }
    }

    /// Sets the application-wide context of type `T`: what `context!()`
    /// reads where no ancestor provides a `T`.
    ///
    /// Can be called before the first frame, to hand the root component
    /// something it cannot take as props, or at any time after; a changed
    /// value updates only the components that read it.
    pub fn provide_context<T: PartialEq + 'static>(&mut self, value: T) {
        self.reactive.provide_context(value);
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
        // A canvas invalidated from a timer or a worker has no other signal to
        // ride in on, so it wakes the loop through the same channel async state
        // updates do.
        let wake = std::sync::Arc::new(wake);
        #[cfg(feature = "hotpatch")]
        crate::hot::set_wake(wake.clone());
        let canvas_wake = wake.clone();
        self.ui_runtime
            .canvas_invalidator()
            .set_wake(std::rc::Rc::new(move || canvas_wake()));
        let scroll_wake = wake.clone();
        self.ui_runtime
            .scroll_request_queue()
            .set_wake(std::rc::Rc::new(move || scroll_wake()));
        self.reactive.tasks().set_wake(wake);
    }

    /// Physical pixels per logical pixel. Canvas painters use it to size
    /// hairlines and snap to the device grid.
    /// Hands the runtime the renderer's GPU device, so canvases built with
    /// [`crate::widgets::CanvasController::with_gpu_painter`] can draw on it.
    ///
    /// Called once by the renderer at startup. Until it is, a GPU painter
    /// draws nothing rather than failing.
    pub fn set_gpu_context(&mut self, context: crate::widgets::CanvasGpuContext) {
        self.ui_runtime.set_gpu_context(context);
    }

    pub fn set_scale_factor(&mut self, scale_factor: f32) {
        self.ui_runtime.set_scale_factor(scale_factor);
        self.reactive
            .provide_keyed::<system::viewport::ScaleFactor>(scale_factor);
    }

    /// Writes the results of the `resource!` futures that finished since
    /// the last call; whether there were any.
    pub fn drain_async_messages(&mut self) -> bool {
        let mut any = false;
        #[cfg(feature = "hotpatch")]
        if crate::hot::take_patched() {
            self.hot_remount();
            any = true;
        }
        while let Ok(finished) = self.finished.try_recv() {
            self.reactive.finish_task(finished);
            any = true;
        }
        any
    }

    pub fn resize(&mut self, size: Size<f32>) {
        if self.size != size {
            self.size = size;
            self.ui_runtime
                .mark_subtree_layout_dirty(self.ui_runtime.root());
            self.reactive
                .provide_keyed::<system::viewport::LogicalSize>(size);
        }
    }

    pub fn dispatch_event<T: TextBackend>(
        &mut self,
        event: RawEvent,
        text: &mut TextHost<T>,
    ) -> EventResult {
        match &event {
            RawEvent::WindowFocus(_) => {
                self.reactive.provide_keyed::<system::window::Focused>(true)
            }
            RawEvent::WindowBlur(_) => self
                .reactive
                .provide_keyed::<system::window::Focused>(false),
            _ => {}
        }
        self.rebuild_sync_if_needed();
        self.ui_runtime.update_tree(self.size, text);
        // Before the input: a clamp that layout just made happened first.
        self.dispatch_clamped_scrolls(text);
        self.ui_runtime
            .dispatch_event(text, &mut self.event_translator, event)
    }

    /// A drag from outside the window, offered to the drop target under it.
    ///
    /// The target it left hears `DropLeave` and the one it reached `DropEnter`;
    /// while it stays on one, that one hears `DropOver`. On `Drop` the target
    /// under the drop gets it, and the drag is over either way.
    pub(crate) fn dispatch_external_drag<T: TextBackend>(
        &mut self,
        event: xui_interface::DragEvent,
        text: &mut TextHost<T>,
    ) -> EventResult {
        use xui_interface::DragEvent as Native;

        self.rebuild_sync_if_needed();
        self.ui_runtime.update_tree(self.size, text);
        let now = std::time::Instant::now();
        match event {
            Native::Enter { payload, position } => {
                // An enter with a drag still on record: the host lost a leave.
                self.end_external_drag(now, text);
                *self.ui_runtime.external_drag_mut() = Some(ExternalDrag {
                    payload: Arc::new(payload),
                    position,
                    target: None,
                });
                self.move_external_drag(position, now, text)
            }
            Native::Over { position } => self.move_external_drag(position, now, text),
            Native::Leave => {
                self.end_external_drag(now, text);
                EventResult::Ignored
            }
            Native::Drop { payload, position } => {
                // The payload as of the drop, which is what the source
                // promised; an enter the host never sent is no reason to lose it.
                let payload = Arc::new(payload);
                let drag = self.ui_runtime.external_drag_mut();
                match drag {
                    Some(drag) => drag.payload = payload,
                    None => {
                        *drag = Some(ExternalDrag {
                            payload,
                            position,
                            target: None,
                        })
                    }
                }
                self.move_external_drag(position, now, text);
                let Some(drag) = self.ui_runtime.external_drag_mut().take() else {
                    return EventResult::Ignored;
                };
                let Some((target, effect)) = drag.target else {
                    return EventResult::Ignored;
                };
                let local = self.ui_runtime.to_local(target, position);
                let event = self.event_translator.drop_event(
                    SemanticEvent::Drop,
                    target,
                    position,
                    local,
                    drag.payload,
                    effect,
                    now,
                );
                dispatcher::dispatch_semantic(&mut self.ui_runtime, text, event).result
            }
        }
    }

    /// Moves the drag on record to `position`, telling whichever targets that
    /// concerns.
    fn move_external_drag<T: TextBackend>(
        &mut self,
        position: Point,
        now: std::time::Instant,
        text: &mut TextHost<T>,
    ) -> EventResult {
        let Some(drag) = self.ui_runtime.external_drag_mut().as_mut() else {
            return EventResult::Ignored;
        };
        drag.position = position;

        let payload = Arc::clone(&drag.payload);
        let previous = drag.target;
        let next = self.ui_runtime.drop_target_at(position, payload.kinds());
        let node = |target: Option<(NodeId, DropEffect)>| target.map(|(node, _)| node);

        let mut result = EventResult::Ignored;

        let mut send = |app: &mut Self, kind: fn(DropEvent) -> SemanticEvent, (node, effect)| {
            let local = app.ui_runtime.to_local(node, position);
            let event = app.event_translator.drop_event(
                kind,
                node,
                position,
                local,
                Arc::clone(&payload),
                effect,
                now,
            );
            if dispatcher::dispatch_semantic(&mut app.ui_runtime, text, event)
                .result
                .is_consumed()
            {
                result = EventResult::Consumed;
            }
        };

        if node(previous) == node(next) {
            if let Some(target) = next {
                send(self, SemanticEvent::DropOver, target);
            }
            return result;
        }

        if let Some(target) = previous {
            send(self, SemanticEvent::DropLeave, target);
        }
        // Recorded before the enter is sent: a handler that removes the
        // target clears it through the commit, like any other removal.
        if let Some(drag) = self.ui_runtime.external_drag_mut() {
            drag.target = next;
        }
        if let Some(target) = next {
            send(self, SemanticEvent::DropEnter, target);
        }
        result
    }

    /// Forgets the drag on record, telling the target it was over.
    fn end_external_drag<T: TextBackend>(
        &mut self,
        now: std::time::Instant,
        text: &mut TextHost<T>,
    ) {
        let Some(drag) = self.ui_runtime.external_drag_mut().take() else {
            return;
        };
        let Some((node, effect)) = drag.target else {
            return;
        };
        let local = self.ui_runtime.to_local(node, drag.position);
        let event = self.event_translator.drop_event(
            SemanticEvent::DropLeave,
            node,
            drag.position,
            local,
            drag.payload,
            effect,
            now,
        );
        dispatcher::dispatch_semantic(&mut self.ui_runtime, text, event);
    }

    /// Publishes the frame's time, before anything in it is stepped or drawn.
    ///
    /// Everything that animates during the frame -- style transitions, canvas
    /// painters, and the hooks that will be layered on top of them -- reads
    /// this one value rather than a clock of its own, so they cannot disagree
    /// about when the frame is. Driven by [`crate::runtime::GuiRuntime`]; an
    /// embedder running its own loop calls it once per frame, or leaves it
    /// alone and gets [`FrameTime::ZERO`] throughout.
    #[inline(always)]
    pub fn begin_frame(&mut self, frame: FrameTime) {
        self.ui_runtime.begin_frame(frame);
    }

    /// The time published by the last [`Self::begin_frame`].
    #[inline(always)]
    pub fn frame_time(&self) -> FrameTime {
        self.ui_runtime.frame_time()
    }

    #[inline(always)]
    pub fn tick_style_animations(&mut self, delta: Duration) -> bool {
        self.ui_runtime.tick_style_animations(delta)
    }

    /// Tells the runtime whether the window is on screen, which stops the
    /// animation loop while it is not. See
    /// [`crate::ui_runtime::UiRuntime::set_window_visible`].
    ///
    /// Not public: stopping the loop without also stopping the frame clock
    /// would turn the pause into a jump on the way back, and the clock belongs
    /// to the runtime driving the loop. Go through
    /// [`crate::runtime::GuiRuntime::set_window_visible`].
    #[inline(always)]
    pub(crate) fn set_window_visible(&mut self, visible: bool) {
        self.ui_runtime.set_window_visible(visible);
        self.reactive
            .provide_keyed::<system::window::Visible>(visible);
    }

    /// What a component reading `context!(K)` would see now.
    #[cfg(test)]
    pub(crate) fn system<K: crate::reactive::ContextKey>(&self) -> K::Value
    where
        K::Value: Clone,
    {
        self.reactive.keyed::<K>()
    }

    /// Records whether the window is minimized, for `window::Minimized`.
    pub(crate) fn set_window_minimized(&mut self, minimized: bool) {
        self.reactive
            .provide_keyed::<system::window::Minimized>(minimized);
    }

    /// Records the fullscreen and maximized state someone other than the
    /// runtime set, on the platform handle and as `window::Fullscreen` and
    /// `window::Maximized`.
    pub(crate) fn set_window_state(&mut self, state: xui_interface::WindowState) {
        self.platform.set_window_state(state);
        self.reactive
            .provide_keyed::<system::window::Fullscreen>(state.fullscreen);
        self.reactive
            .provide_keyed::<system::window::Maximized>(state.maximized);
    }

    #[inline(always)]
    pub fn window_visible(&self) -> bool {
        self.ui_runtime.window_visible()
    }

    /// Marks every canvas that asked to animate dirty for this frame.
    ///
    /// Called once a frame beside [`Self::tick_style_animations`], for the same
    /// reason: the work is scheduled by being dirty, and something has to
    /// re-dirty it each frame for the loop to continue.
    pub fn tick_animating_canvases(&mut self) {
        self.ui_runtime.tick_animating_canvases();
    }

    /// Runs the callbacks installed by
    /// [`crate::state::HookContext::use_ticker`], once for this frame.
    ///
    /// The third of the frame's animation steps, beside
    /// [`Self::tick_style_animations`] and [`Self::tick_animating_canvases`].
    /// Runs before the tree is rebuilt, so anything a ticker invalidates is
    /// part of this frame rather than the one after it.
    pub fn tick_tickers(&mut self, frame: FrameTime) {
        self.ui_runtime.tick_tickers(frame);
    }

    pub fn has_running_style_animations(&self) -> bool {
        self.ui_runtime.has_running_style_animations()
    }

    pub fn render<B: RenderBackend<TextHost<T>>, T: TextBackend>(
        &mut self,
        backend: &mut B,
        text: &mut TextHost<T>,
    ) -> Result<(), AppRenderError<B::Error>> {
        self.drain_async_messages();
        self.flush_scroll_requests(text);

        // Every build runs to completion within its frame: a frame never
        // paints a tree some of whose updates are still pending. Rendering a
        // component costs about a microsecond, so the component phase only
        // outgrows a frame for builds in the thousands of components -- and
        // an input event already finishes any pending build before it is
        // handled (`dispatch_event`), so slicing bought no responsiveness.
        self.rebuild_sync_if_needed();
        self.flush_node_lifecycle(backend, text);
        let committed = true;

        let drawn = self.draw(backend, text);
        // By now a widget the commit unmounted and the frame that last drew it
        // have both let go of their assets, so released ones can be told apart
        // from ones still in use.
        if committed {
            crate::assets::collect_released();
        }
        drawn
    }

    /// Applies `ScrollController` requests and reports each as a `Scroll`
    /// event.
    ///
    /// Runs before the rebuild, not inside `update_tree`, for two reasons.
    /// Dispatching needs the translator, which the host layer does not own.
    /// And a handler that answers the scroll with a state change -- a virtual
    /// list mounting the rows it jumped to -- gets that change committed in
    /// this same frame instead of showing the gap for one.
    fn flush_scroll_requests<T: TextBackend>(&mut self, text: &mut TextHost<T>) {
        if !self.ui_runtime.has_pending_scroll_requests() {
            return;
        }
        // Requests resolve against layout, so bring it up to date first: a
        // `scroll_to_end` after adding content has to see that content.
        self.ui_runtime.update_tree(self.size, text);
        self.dispatch_clamped_scrolls(text);
        let applied = self.ui_runtime.apply_scroll_requests();
        self.dispatch_applied_scrolls(applied, ScrollSource::Programmatic, text);
    }

    /// Reports the offsets layout moved back into range as `Scroll` events
    /// with `ScrollSource::Layout`, so a listener tracking the offset -- a
    /// virtual list choosing its rows -- does not keep a stale one.
    ///
    /// Called after every `update_tree` the app runs, which is every place a
    /// layout can have clamped something.
    fn dispatch_clamped_scrolls<T: TextBackend>(&mut self, text: &TextHost<T>) {
        let clamped = self.ui_runtime.take_clamped_scrolls();
        self.dispatch_applied_scrolls(clamped, ScrollSource::Layout, text);
    }

    fn dispatch_applied_scrolls<T: TextBackend>(
        &mut self,
        scrolls: Vec<AppliedScroll>,
        source: ScrollSource,
        text: &TextHost<T>,
    ) {
        if scrolls.is_empty() {
            return;
        }
        let now = std::time::Instant::now();
        let ui_runtime = &mut self.ui_runtime;
        let translator = &mut self.event_translator;
        for scroll in scrolls {
            let event = translator.scroll_offset_event(
                scroll,
                source,
                ScrollPhase::Move,
                now,
                xui_interface::Modifiers::default(),
                None,
            );
            dispatcher::dispatch_semantic(ui_runtime, text, event);
        }
    }

    /// Scrolls for a navigation key -- arrows, Page Up/Down, Home/End, Space --
    /// that nothing else claimed.
    ///
    /// This is a default action, so it must run last: [`GuiRuntime`] calls it
    /// only once widgets and shortcuts have declined the key, which is what
    /// keeps a focused tab list's arrows or a text input's Home/End from also
    /// scrolling the container around them.
    ///
    /// [`GuiRuntime`]: crate::runtime::GuiRuntime
    pub fn dispatch_keyboard_scroll<T: TextBackend>(
        &mut self,
        raw: &xui_interface::RawKeyboard,
        text: &mut TextHost<T>,
    ) -> EventResult {
        let events = self
            .event_translator
            .translate_keyboard_scroll(raw, &mut self.ui_runtime);
        if events.is_empty() {
            return EventResult::Ignored;
        }
        let ui_runtime = &mut self.ui_runtime;
        for event in events {
            dispatcher::dispatch_semantic(ui_runtime, text, event);
        }
        EventResult::Consumed
    }

    fn draw<B: RenderBackend<TextHost<T>>, T: TextBackend>(
        &mut self,
        backend: &mut B,
        text: &mut TextHost<T>,
    ) -> Result<(), AppRenderError<B::Error>> {
        let drawn = self.draw_frame(backend, text);
        // After the frame, not between its layout and its build: a handler may
        // invalidate nodes, and the frame being built must not see that half
        // done. Whatever it changes is picked up by the next frame, which
        // `is_dirty` then asks for.
        self.dispatch_clamped_scrolls(text);
        drawn
    }

    fn draw_frame<B: RenderBackend<TextHost<T>>, T: TextBackend>(
        &mut self,
        backend: &mut B,
        text: &mut TextHost<T>,
    ) -> Result<(), AppRenderError<B::Error>> {
        self.ui_runtime.update_tree(self.size, text);
        // Against the layout just settled, which is what this frame shows.
        self.platform
            .set_drop_targets(self.ui_runtime.drop_targets());

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
        self.reactive.is_dirty() || self.ui_runtime.is_dirty() || self.theme.is_requested()
    }

    /// Asks for a frame that lays out and paints everything again: after a
    /// change the runtime cannot see, such as the scale factor.
    pub fn mark_needs_rebuild(&mut self) {
        self.ui_runtime
            .mark_subtree_layout_dirty(self.ui_runtime.root());
    }

    #[inline]
    fn rebuild_sync_if_needed(&mut self) {
        // What handlers wrote since the last flush.
        if self.reactive.is_dirty() {
            self.reactive.flush(&mut self.ui_runtime);
        }
        // After the flush, so a theme an effect switched restyles this frame.
        if self.theme.is_requested() {
            self.apply_theme();
        }
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

/// What [`crate::system`]'s keys read before the host reports anything.
fn provide_system_defaults(reactive: &mut crate::reactive::Runtime) {
    use system::{viewport, window};
    reactive.provide_keyed::<window::Fullscreen>(false);
    reactive.provide_keyed::<window::Maximized>(false);
    reactive.provide_keyed::<window::Minimized>(false);
    reactive.provide_keyed::<window::Focused>(true);
    reactive.provide_keyed::<window::Visible>(true);
    reactive.provide_keyed::<viewport::LogicalSize>(Size::<f32>::ZERO);
    reactive.provide_keyed::<viewport::ScaleFactor>(1.0);
}
