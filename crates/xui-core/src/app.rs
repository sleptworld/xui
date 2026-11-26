use crate::clock::FrameTime;
use crate::core::Size;
use crate::event_system::dispatcher;
use crate::platform::PlatformHandle;
use crate::render::RenderBackend;
use crate::scroll::AppliedScroll;
use crate::shortcut::ShortcutManager;
use crate::style::Theme;
use crate::system;
use crate::text::TextHost;
use crate::theme::ThemeHandle;
use crate::ui_runtime::{ExternalDrag, RenderFrameError, UiRuntime};
use crate::windows::{WindowChange, Windows};
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
use xui_interface::{
    Appearance, DropEffect, DropEvent, NodeId, Point, ThemeMode, Themes, WindowId,
};

type Wake = Arc<dyn Fn() + Send + Sync>;

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
    /// Each window's tree, translator and size; only the main one for now.
    windows: Windows,
    reactive: crate::reactive::Runtime,
    tokio_runtime: TokioRuntime,
    /// Results of `resource!` futures, waiting for the main thread.
    finished: std::sync::mpsc::Receiver<(u64, Box<dyn std::any::Any + Send>)>,
    platform: PlatformHandle,
    /// What [`Self::set_async_wake_callback`] was given, for the trees of the
    /// windows opened after it.
    wake: std::cell::RefCell<Option<Wake>>,
    /// What [`Self::set_gpu_context`] was given, for the windows opened after it.
    gpu_context: Option<crate::widgets::CanvasGpuContext>,
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

    /// Window `window`'s handle: what it sends is its own, and what it sets
    /// -- title, menu, surface -- is that window's.
    pub fn window_platform(&self, window: WindowId) -> PlatformHandle {
        self.platform.for_window(window)
    }

    /// Raises a command the platform itself originated -- a menu item, or one
    /// of the system's standard actions -- at the focused node, from where it
    /// bubbles like any command. With nothing focused it goes to the command
    /// root, as a global shortcut does.
    pub(crate) fn dispatch_platform_command<T: TextBackend>(
        &mut self,
        window: WindowId,
        command: xui_interface::CommandId,
        timestamp: std::time::Instant,
        text: &mut TextHost<T>,
    ) -> EventResult {
        let target = self.windows[window]
            .ui
            .focus_manager()
            .focused()
            .unwrap_or_else(|| self.command_root(window));
        let event = self.windows[window]
            .event_translator
            .platform_command_event(
                target,
                command,
                timestamp,
                xui_interface::Modifiers::default(),
            );
        let result = crate::event_system::dispatcher::dispatch_semantic(
            &mut self.windows[window].ui,
            text,
            event,
        )
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
        window: WindowId,
        now: std::time::Instant,
        text: &mut TextHost<T>,
    ) {
        let due = self.windows[window].ui.take_due_wakes(now);
        if due.is_empty() {
            return;
        }
        for (node, deadline) in due {
            let event = self.windows[window]
                .event_translator
                .wake_event(node, deadline, now);
            crate::event_system::dispatcher::dispatch_semantic(
                &mut self.windows[window].ui,
                text,
                event,
            );
        }
    }

    /// When the next frame is owed to a wake, if one is. See
    /// [`crate::runtime::GuiRuntime::next_wake`].
    pub fn next_wake(&self, window: WindowId) -> Option<std::time::Instant> {
        self.windows[window].ui.next_wake()
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
            // To the window the node is in; a node gone since is dropped.
            let Some(window) = self.windows.try_window_of(reply.node) else {
                continue;
            };
            let state = &mut self.windows[window];
            let event = state.event_translator.platform_reply_event(
                reply.node,
                reply.request,
                reply.reply,
                timestamp,
            );
            let report =
                crate::event_system::dispatcher::dispatch_semantic(&mut state.ui, text, event);
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
        window: WindowId,
        event: &xui_interface::RawKeyboard,
    ) -> Option<(xui_interface::NodeId, xui_interface::ShortcutBinding)> {
        self.windows[window]
            .ui
            .resolve_local_shortcut(event)
            .or_else(|| {
                self.shortcuts
                    .resolve(event)
                    .map(|binding| (self.command_root(window), binding))
            })
    }

    pub(crate) fn command_root(&self, window: WindowId) -> xui_interface::NodeId {
        let ui = &self.windows[window].ui;
        ui.children(ui.root())
            .find(|child| *child != ui.root_overlayer())
            .unwrap_or(ui.root())
    }

    pub(crate) fn dispatch_command<T: TextBackend>(
        &mut self,
        window: WindowId,
        target: xui_interface::NodeId,
        binding: xui_interface::ShortcutBinding,
        raw: &xui_interface::RawKeyboard,
        text: &mut TextHost<T>,
    ) -> EventResult {
        let event = self.windows[window].event_translator.command_event(
            target,
            binding.command,
            binding.shortcut,
            raw,
        );
        let result = crate::event_system::dispatcher::dispatch_semantic(
            &mut self.windows[window].ui,
            text,
            event,
        )
        .result;
        result
    }
    /// An app set up by `root`, which mounts what it shows.
    pub fn new(root: AppRoot) -> Self {
        let mut windows = Windows::new();
        let main = windows.main_id();
        let ui_runtime = &mut windows.main_mut().ui;
        let platform = PlatformHandle::new(main);
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
        reactive.provide_window_defaults(main);
        ui_runtime.set_theme(theme.current());
        let mut app = Self {
            windows,
            reactive,
            tokio_runtime,
            finished,
            platform,
            wake: std::cell::RefCell::new(None),
            gpu_context: None,
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

    /// The window the host made at startup, which [`Self::mount`] mounts into.
    pub fn main_window(&self) -> WindowId {
        self.windows.main_id()
    }

    /// Every open window, the main one included: the main window and those
    /// the mounted window blocks opened.
    pub fn window_ids(&self) -> Vec<WindowId> {
        self.windows.ids().collect()
    }

    /// Window `window`'s tree; `window` must be open.
    pub fn window_ui(&self, window: WindowId) -> &UiRuntime {
        &self.windows[window].ui
    }

    /// The main window's tree.
    pub fn ui_runtime(&self) -> &UiRuntime {
        &self.windows.main().ui
    }

    /// Mounts reactive component `C` at the root, and runs its first effects.
    pub fn mount<C: crate::reactive::Component>(
        &mut self,
        props: <C::Instance as crate::reactive::ComponentDef>::Props,
    ) -> crate::reactive::InstanceId {
        let root = self.windows.main().ui.root();
        let id = self.reactive.mount::<C>(&mut self.windows, root, props);
        self.apply_window_changes();
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
            self.reactive.hot_reload(&mut self.windows)
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
                    self.reactive.unmount(&mut self.windows, id);
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
        for window in self.windows.ids().collect::<Vec<_>>() {
            self.mark_needs_rebuild(window);
        }
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
        self.reactive.unmount(&mut self.windows, id);
        self.apply_window_changes();
    }

    #[cfg(test)]
    pub(crate) fn ui_runtime_mut(&mut self) -> &mut UiRuntime {
        &mut self.windows.main_mut().ui
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
            for ui in self.windows.uis_mut() {
                ui.set_theme(theme.clone());
            }
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
        let wake: Wake = Arc::new(wake);
        #[cfg(feature = "hotpatch")]
        crate::hot::set_wake(wake.clone());
        connect_wake(&self.windows.main().ui, &wake);
        self.reactive.tasks().set_wake(wake.clone());
        *self.wake.borrow_mut() = Some(wake);
    }

    /// Tells the host about the windows window blocks opened and closed, and
    /// sets the trees of the new ones up as the main window's was.
    fn apply_window_changes(&mut self) {
        for change in self.windows.take_changes() {
            match change {
                WindowChange::Opened(window, options) => {
                    // Gone already if it closed in the same flush.
                    if self.windows.get(window).is_some() {
                        // Not on screen until a host has made it and shown its
                        // first frame; a host that cannot never does.
                        self.set_window_visible(window, false);
                        let ui = self.windows.ui_mut(window);
                        ui.set_platform(self.platform.for_window(window));
                        ui.set_theme(self.theme.current());
                        if let Some(wake) = &*self.wake.borrow() {
                            connect_wake(ui, wake);
                        }
                        if let Some(context) = &self.gpu_context {
                            ui.set_gpu_context(context.clone());
                        }
                    }
                    self.platform
                        .send(xui_interface::PlatformCommand::OpenWindow { window, options });
                }
                WindowChange::Closed(window) => {
                    self.platform.forget_window(window);
                    self.platform
                        .send(xui_interface::PlatformCommand::CloseWindow(window));
                }
            }
        }
    }

    /// The user asked to close `window`, with its close button or a key.
    ///
    /// The window block that opened it runs its `on_close_requested`, which
    /// closes it by writing what keeps the block mounted; without one, the
    /// window is hidden and kept, `window::Visible` false. The main window
    /// is the host's: asking closes nothing here.
    pub fn request_close(&mut self, window: WindowId) {
        let Some(state) = self.windows.get(window) else {
            return;
        };
        if window == self.windows.main_id() {
            return;
        }
        match state.on_close_requested.clone() {
            Some(handler) => handler(),
            None => self
                .reactive
                .provide_keyed::<system::window::Visible>(window, false),
        }
    }

    /// Physical pixels per logical pixel. Canvas painters use it to size
    /// hairlines and snap to the device grid.
    /// Hands the runtime the renderer's GPU device, so canvases built with
    /// [`crate::widgets::CanvasController::with_gpu_painter`] can draw on it.
    ///
    /// Called once by the renderer at startup. Until it is, a GPU painter
    /// draws nothing rather than failing.
    pub fn set_gpu_context(&mut self, context: crate::widgets::CanvasGpuContext) {
        for ui in self.windows.uis_mut() {
            ui.set_gpu_context(context.clone());
        }
        self.gpu_context = Some(context);
    }

    pub fn set_scale_factor(&mut self, window: WindowId, scale_factor: f32) {
        self.windows[window].ui.set_scale_factor(scale_factor);
        self.reactive
            .provide_keyed::<system::viewport::ScaleFactor>(window, scale_factor);
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

    pub fn resize(&mut self, window: WindowId, size: Size<f32>) {
        if self.windows[window].size != size {
            let state = &mut self.windows[window];
            state.size = size;
            state.ui.mark_subtree_layout_dirty(state.ui.root());
            self.reactive
                .provide_keyed::<system::viewport::LogicalSize>(window, size);
        }
    }

    pub fn dispatch_event<T: TextBackend>(
        &mut self,
        window: WindowId,
        event: RawEvent,
        text: &mut TextHost<T>,
    ) -> EventResult {
        match &event {
            RawEvent::WindowFocus(_) => self
                .reactive
                .provide_keyed::<system::window::Focused>(window, true),
            RawEvent::WindowBlur(_) => self
                .reactive
                .provide_keyed::<system::window::Focused>(window, false),
            _ => {}
        }
        self.rebuild_sync_if_needed();
        {
            let state = &mut self.windows[window];
            state.ui.update_tree(state.size, text);
        }
        // Before the input: a clamp that layout just made happened first.
        self.dispatch_clamped_scrolls(window, text);
        let state = &mut self.windows[window];
        state
            .ui
            .dispatch_event(text, &mut state.event_translator, event)
    }

    /// A drag from outside the window, offered to the drop target under it.
    ///
    /// The target it left hears `DropLeave` and the one it reached `DropEnter`;
    /// while it stays on one, that one hears `DropOver`. On `Drop` the target
    /// under the drop gets it, and the drag is over either way.
    pub(crate) fn dispatch_external_drag<T: TextBackend>(
        &mut self,
        window: WindowId,
        event: xui_interface::DragEvent,
        text: &mut TextHost<T>,
    ) -> EventResult {
        use xui_interface::DragEvent as Native;

        self.rebuild_sync_if_needed();
        {
            let state = &mut self.windows[window];
            state.ui.update_tree(state.size, text);
        }
        let now = std::time::Instant::now();
        match event {
            Native::Enter { payload, position } => {
                // An enter with a drag still on record: the host lost a leave.
                self.end_external_drag(window, now, text);
                *self.windows[window].ui.external_drag_mut() = Some(ExternalDrag {
                    payload: Arc::new(payload),
                    position,
                    target: None,
                });
                self.move_external_drag(window, position, now, text)
            }
            Native::Over { position } => self.move_external_drag(window, position, now, text),
            Native::Leave => {
                self.end_external_drag(window, now, text);
                EventResult::Ignored
            }
            Native::Drop { payload, position } => {
                // The payload as of the drop, which is what the source
                // promised; an enter the host never sent is no reason to lose it.
                let payload = Arc::new(payload);
                let drag = self.windows[window].ui.external_drag_mut();
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
                self.move_external_drag(window, position, now, text);
                let Some(drag) = self.windows[window].ui.external_drag_mut().take() else {
                    return EventResult::Ignored;
                };
                let Some((target, effect)) = drag.target else {
                    return EventResult::Ignored;
                };
                let local = self.windows[window].ui.to_local(target, position);
                let event = self.windows[window].event_translator.drop_event(
                    SemanticEvent::Drop,
                    target,
                    position,
                    local,
                    drag.payload,
                    effect,
                    now,
                );
                dispatcher::dispatch_semantic(&mut self.windows[window].ui, text, event).result
            }
        }
    }

    /// Moves the drag on record to `position`, telling whichever targets that
    /// concerns.
    fn move_external_drag<T: TextBackend>(
        &mut self,
        window: WindowId,
        position: Point,
        now: std::time::Instant,
        text: &mut TextHost<T>,
    ) -> EventResult {
        let Some(drag) = self.windows[window].ui.external_drag_mut().as_mut() else {
            return EventResult::Ignored;
        };
        drag.position = position;

        let payload = Arc::clone(&drag.payload);
        let previous = drag.target;
        let next = self.windows[window]
            .ui
            .drop_target_at(position, payload.kinds());
        let node = |target: Option<(NodeId, DropEffect)>| target.map(|(node, _)| node);

        let mut result = EventResult::Ignored;

        let mut send = |app: &mut Self, kind: fn(DropEvent) -> SemanticEvent, (node, effect)| {
            let local = app.windows[window].ui.to_local(node, position);
            let event = app.windows[window].event_translator.drop_event(
                kind,
                node,
                position,
                local,
                Arc::clone(&payload),
                effect,
                now,
            );
            if dispatcher::dispatch_semantic(&mut app.windows[window].ui, text, event)
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
        if let Some(drag) = self.windows[window].ui.external_drag_mut() {
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
        window: WindowId,
        now: std::time::Instant,
        text: &mut TextHost<T>,
    ) {
        let Some(drag) = self.windows[window].ui.external_drag_mut().take() else {
            return;
        };
        let Some((node, effect)) = drag.target else {
            return;
        };
        let local = self.windows[window].ui.to_local(node, drag.position);
        let event = self.windows[window].event_translator.drop_event(
            SemanticEvent::DropLeave,
            node,
            drag.position,
            local,
            drag.payload,
            effect,
            now,
        );
        dispatcher::dispatch_semantic(&mut self.windows[window].ui, text, event);
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
    pub fn begin_frame(&mut self, window: WindowId, frame: FrameTime) {
        self.windows[window].ui.begin_frame(frame);
    }

    /// The time published by the last [`Self::begin_frame`].
    #[inline(always)]
    pub fn frame_time(&self, window: WindowId) -> FrameTime {
        self.windows[window].ui.frame_time()
    }

    #[inline(always)]
    pub fn tick_style_animations(&mut self, window: WindowId, delta: Duration) -> bool {
        self.windows[window].ui.tick_style_animations(delta)
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
    pub(crate) fn set_window_visible(&mut self, window: WindowId, visible: bool) {
        self.windows[window].ui.set_window_visible(visible);
        self.reactive
            .provide_keyed::<system::window::Visible>(window, visible);
    }

    /// What a component reading `context!(K)` would see now.
    #[cfg(test)]
    pub(crate) fn system<K: crate::reactive::ContextKey>(&self) -> K::Value
    where
        K::Value: Clone,
    {
        self.reactive.keyed::<K>(self.windows.main_id())
    }

    /// Records whether the window is minimized, for `window::Minimized`.
    pub(crate) fn set_window_minimized(&mut self, window: WindowId, minimized: bool) {
        self.reactive
            .provide_keyed::<system::window::Minimized>(window, minimized);
    }

    /// Records the fullscreen and maximized state someone other than the
    /// runtime set, on the platform handle and as `window::Fullscreen` and
    /// `window::Maximized`.
    pub(crate) fn set_window_state(&mut self, window: WindowId, state: xui_interface::WindowState) {
        self.platform.for_window(window).set_window_state(state);
        self.reactive
            .provide_keyed::<system::window::Fullscreen>(window, state.fullscreen);
        self.reactive
            .provide_keyed::<system::window::Maximized>(window, state.maximized);
    }

    #[inline(always)]
    pub fn window_visible(&self, window: WindowId) -> bool {
        self.windows[window].ui.window_visible()
    }

    /// Marks every canvas that asked to animate dirty for this frame.
    ///
    /// Called once a frame beside [`Self::tick_style_animations`], for the same
    /// reason: the work is scheduled by being dirty, and something has to
    /// re-dirty it each frame for the loop to continue.
    pub fn tick_animating_canvases(&mut self, window: WindowId) {
        self.windows[window].ui.tick_animating_canvases();
    }

    /// Runs the callbacks installed by
    /// [`crate::state::HookContext::use_ticker`], once for this frame.
    ///
    /// The third of the frame's animation steps, beside
    /// [`Self::tick_style_animations`] and [`Self::tick_animating_canvases`].
    /// Runs before the tree is rebuilt, so anything a ticker invalidates is
    /// part of this frame rather than the one after it.
    pub fn tick_tickers(&mut self, window: WindowId, frame: FrameTime) {
        self.windows[window].ui.tick_tickers(frame);
    }

    pub fn has_running_style_animations(&self, window: WindowId) -> bool {
        self.windows[window].ui.has_running_style_animations()
    }

    pub fn render<B: RenderBackend<TextHost<T>>, T: TextBackend>(
        &mut self,
        window: WindowId,
        backend: &mut B,
        text: &mut TextHost<T>,
    ) -> Result<(), AppRenderError<B::Error>> {
        self.drain_async_messages();
        self.flush_scroll_requests(window, text);

        // Every build runs to completion within its frame: a frame never
        // paints a tree some of whose updates are still pending. Rendering a
        // component costs about a microsecond, so the component phase only
        // outgrows a frame for builds in the thousands of components -- and
        // an input event already finishes any pending build before it is
        // handled (`dispatch_event`), so slicing bought no responsiveness.
        self.rebuild_sync_if_needed();
        self.flush_node_lifecycle(window, backend, text);
        let committed = true;

        let drawn = self.draw(window, backend, text);
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
    fn flush_scroll_requests<T: TextBackend>(&mut self, window: WindowId, text: &mut TextHost<T>) {
        if !self.windows[window].ui.has_pending_scroll_requests() {
            return;
        }
        // Requests resolve against layout, so bring it up to date first: a
        // `scroll_to_end` after adding content has to see that content.
        {
            let state = &mut self.windows[window];
            state.ui.update_tree(state.size, text);
        }
        self.dispatch_clamped_scrolls(window, text);
        let applied = self.windows[window].ui.apply_scroll_requests();
        self.dispatch_applied_scrolls(window, applied, ScrollSource::Programmatic, text);
    }

    /// Reports the offsets layout moved back into range as `Scroll` events
    /// with `ScrollSource::Layout`, so a listener tracking the offset -- a
    /// virtual list choosing its rows -- does not keep a stale one.
    ///
    /// Called after every `update_tree` the app runs, which is every place a
    /// layout can have clamped something.
    fn dispatch_clamped_scrolls<T: TextBackend>(&mut self, window: WindowId, text: &TextHost<T>) {
        let clamped = self.windows[window].ui.take_clamped_scrolls();
        self.dispatch_applied_scrolls(window, clamped, ScrollSource::Layout, text);
    }

    fn dispatch_applied_scrolls<T: TextBackend>(
        &mut self,
        window: WindowId,
        scrolls: Vec<AppliedScroll>,
        source: ScrollSource,
        text: &TextHost<T>,
    ) {
        if scrolls.is_empty() {
            return;
        }
        let now = std::time::Instant::now();
        let state = &mut self.windows[window];
        let (ui_runtime, translator) = (&mut state.ui, &mut state.event_translator);
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
        window: WindowId,
        raw: &xui_interface::RawKeyboard,
        text: &mut TextHost<T>,
    ) -> EventResult {
        let state = &mut self.windows[window];
        let events = state
            .event_translator
            .translate_keyboard_scroll(raw, &mut state.ui);
        if events.is_empty() {
            return EventResult::Ignored;
        }
        let ui_runtime = &mut self.windows[window].ui;
        for event in events {
            dispatcher::dispatch_semantic(ui_runtime, text, event);
        }
        EventResult::Consumed
    }

    fn draw<B: RenderBackend<TextHost<T>>, T: TextBackend>(
        &mut self,
        window: WindowId,
        backend: &mut B,
        text: &mut TextHost<T>,
    ) -> Result<(), AppRenderError<B::Error>> {
        let drawn = self.draw_frame(window, backend, text);
        // After the frame, not between its layout and its build: a handler may
        // invalidate nodes, and the frame being built must not see that half
        // done. Whatever it changes is picked up by the next frame, which
        // `is_dirty` then asks for.
        self.dispatch_clamped_scrolls(window, text);
        drawn
    }

    fn draw_frame<B: RenderBackend<TextHost<T>>, T: TextBackend>(
        &mut self,
        window: WindowId,
        backend: &mut B,
        text: &mut TextHost<T>,
    ) -> Result<(), AppRenderError<B::Error>> {
        {
            let state = &mut self.windows[window];
            state.ui.update_tree(state.size, text);
        }
        // Against the layout just settled, which is what this frame shows.
        let ui = &self.windows[window].ui;
        ui.platform().set_drop_targets(ui.drop_targets());

        let Some(frame) = self.windows[window]
            .ui
            .build_render_frame()
            .map_err(AppRenderError::Frame)?
        else {
            return Ok(());
        };

        backend
            .begin_frame(window, self.windows[window].size)
            .map_err(AppRenderError::Backend)?;
        backend
            .submit(&frame.built, text)
            .map_err(AppRenderError::Backend)?;
        backend.end_frame().map_err(AppRenderError::Backend)?;
        if backend.did_present() {
            self.windows[window].ui.finish_render_frame(&frame);
        }
        Ok(())
    }

    pub fn is_dirty(&self) -> bool {
        self.reactive.is_dirty()
            || self.theme.is_requested()
            || self.windows.uis().any(UiRuntime::is_dirty)
    }

    /// Asks for a frame that lays out and paints everything again: after a
    /// change the runtime cannot see, such as the scale factor.
    pub fn mark_needs_rebuild(&mut self, window: WindowId) {
        let ui = &mut self.windows[window].ui;
        ui.mark_subtree_layout_dirty(ui.root());
    }

    /// Whether window `window` owes a frame: something in it changed, or a
    /// write waits that may change it.
    pub fn window_is_dirty(&self, window: WindowId) -> bool {
        self.reactive.is_dirty() || self.theme.is_requested() || self.windows[window].ui.is_dirty()
    }

    #[inline]
    fn rebuild_sync_if_needed(&mut self) {
        // What handlers wrote since the last flush.
        if self.reactive.is_dirty() {
            self.reactive.flush(&mut self.windows);
        }
        // Whatever mounted or unmounted a window block since the last sync.
        self.apply_window_changes();
        // After the flush, so a theme an effect switched restyles this frame.
        if self.theme.is_requested() {
            self.apply_theme();
        }
    }

    fn flush_node_lifecycle<B: RenderBackend<TextHost<T>>, T: TextBackend>(
        &mut self,
        window: WindowId,
        backend: &mut B,
        text: &mut TextHost<T>,
    ) {
        for event in self.windows[window].ui.drain_node_lifecycle_events() {
            text.handle_node_lifecycle(&event);
            backend.handle_node_lifecycle(&event);
        }
        self.flush_closed_windows(backend, text);
    }

    /// Tells the text host and the backend about the nodes the windows closed
    /// since the last frame took with them, and lets their trees go.
    fn flush_closed_windows<B: RenderBackend<TextHost<T>>, T: TextBackend>(
        &mut self,
        backend: &mut B,
        text: &mut TextHost<T>,
    ) {
        for mut closed in self.windows.take_closed() {
            for event in closed.ui.drain_node_lifecycle_events() {
                text.handle_node_lifecycle(&event);
                backend.handle_node_lifecycle(&event);
            }
        }
    }
}

/// Wakes the loop when something in `ui` changes from outside a frame: a
/// canvas invalidated from a timer or a worker has no other signal to ride
/// in on, so it wakes the loop through the same channel async state updates do.
fn connect_wake(ui: &UiRuntime, wake: &Wake) {
    let repaint_wake = wake.clone();
    ui.repaint_queue()
        .set_wake(std::rc::Rc::new(move || repaint_wake()));
    let scroll_wake = wake.clone();
    ui.scroll_request_queue()
        .set_wake(std::rc::Rc::new(move || scroll_wake()));
}
