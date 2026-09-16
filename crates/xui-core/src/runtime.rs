use crate::clock::{FrameClock, FrameTime};
use crate::core::Size;
use crate::render::RenderBackend;
use crate::shortcut::ShortcutManager;
use crate::{
    app::{App, AppRenderError},
    text::{TextHost, TextLayoutSlot},
};
use std::collections::VecDeque;
use std::time::Instant;
use xui_interface::{
    EventSource, PlatformOutput, TextBackend as TextBackendI,
    events::{EventResult, RawEvent},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlFlow {
    Poll,
    Wait,
    Exit,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeEvent {
    Resize(Size<f32>),
    Input(RawEvent),
    RedrawRequested,
    Exit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrameReport {
    pub rendered: bool,
    pub event_results: Vec<EventResult>,
}

#[derive(Debug, Default)]
pub struct QueueEventSource {
    events: VecDeque<RuntimeEvent>,
}

impl QueueEventSource {
    pub fn new(events: impl IntoIterator<Item = RuntimeEvent>) -> Self {
        Self {
            events: events.into_iter().collect(),
        }
    }

    pub fn push(&mut self, event: RuntimeEvent) {
        self.events.push_back(event);
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

impl EventSource for QueueEventSource {
    type Event = RuntimeEvent;

    fn poll_event(&mut self) -> Option<Self::Event> {
        self.events.pop_front()
    }
}

pub struct GuiRuntime<B: RenderBackend<TextHost<T>>, T: TextBackendI> {
    app: App,
    backend: B,
    control_flow: ControlFlow,
    text_backend: TextHost<T>,
    frame_clock: FrameClock,
    shortcuts: ShortcutManager,
    platform_output: PlatformOutput,
}

impl<B: RenderBackend<TextHost<T>>, T: TextBackendI> GuiRuntime<B, T> {
    pub fn new(app: App, backend: B, text_backend: T) -> Self {
        Self {
            app,
            backend,
            control_flow: ControlFlow::Poll,
            text_backend: TextHost::new(text_backend),
            frame_clock: FrameClock::new(),
            shortcuts: ShortcutManager::default(),
            platform_output: PlatformOutput::default(),
        }
    }

    pub fn app(&self) -> &App {
        &self.app
    }

    pub fn shortcuts(&self) -> &ShortcutManager {
        &self.shortcuts
    }
    pub fn shortcuts_mut(&mut self) -> &mut ShortcutManager {
        &mut self.shortcuts
    }

    pub fn app_mut(&mut self) -> &mut App {
        &mut self.app
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    pub fn text_backend_mut(&mut self) -> &mut T {
        self.text_backend.backend_mut()
    }

    pub fn control_flow(&self) -> ControlFlow {
        self.control_flow
    }

    pub fn set_control_flow(&mut self, control_flow: ControlFlow) {
        self.control_flow = control_flow;
    }

    /// The clock every animation in this runtime is sampled against.
    pub fn frame_clock(&self) -> &FrameClock {
        &self.frame_clock
    }

    pub fn frame_clock_mut(&mut self) -> &mut FrameClock {
        &mut self.frame_clock
    }

    /// The time of the frame in progress, or of the last one between frames.
    pub fn frame_time(&self) -> FrameTime {
        self.frame_clock.frame()
    }

    pub fn window_visible(&self) -> bool {
        self.app.window_visible()
    }

    /// Tells the runtime whether the window is on screen.
    ///
    /// Two things stop together, which is why this is one call and not two.
    /// The loop stops asking for frames on animation's behalf -- see
    /// [`crate::ui_runtime::UiRuntime::is_animating`] -- and the frame clock
    /// stops, so the time nobody was watching is not time any animation
    /// advances through. Coming back on screen resumes both, from where they
    /// stopped rather than from where a wall clock got to.
    ///
    /// Being told this is best-effort and platform-dependent; a platform that
    /// never reports it simply animates the whole time, which is what every
    /// platform did before this existed.
    pub fn set_window_visible(&mut self, visible: bool) {
        if !visible {
            self.frame_clock.suspend();
        }
        self.app.set_window_visible(visible);
    }

    pub fn platform_output(&self) -> &PlatformOutput {
        &self.platform_output
    }

    fn refresh_platform_output(&mut self) {
        let arena = self.app.ui_runtime();
        let text_input = arena.focus_manager().focused().and_then(|id| {
            let node = arena.node(id)?;
            let rect = arena.visual_layout(id)?;
            let handle = self.text_backend.active_slot(id, TextLayoutSlot::PRIMARY)?;
            let layout = self.text_backend.query(handle)?;
            node.widget.platform_text_input_session(rect, layout)
        });
        let cursor = arena.resolved_cursor();
        self.platform_output = PlatformOutput { text_input, cursor };
    }

    pub fn handle_event(&mut self, event: RuntimeEvent) -> Vec<EventResult> {
        match event {
            RuntimeEvent::Resize(size) => {
                self.app.resize(size);
                Vec::new()
            }
            RuntimeEvent::Input(event) => {
                let keyboard = match &event {
                    RawEvent::Keyboard(raw) => Some(*raw),
                    _ => None,
                };
                let mut result = self.app.dispatch_event(event, &mut self.text_backend);
                if !result.is_consumed()
                    && let Some(raw) = keyboard
                {
                    let resolved = self.app.resolve_local_shortcut(&raw).or_else(|| {
                        self.shortcuts
                            .resolve(&raw)
                            .map(|binding| (self.app.command_root(), binding))
                    });
                    if let Some((target, binding)) = resolved {
                        result = self.app.dispatch_command(
                            target,
                            binding,
                            &raw,
                            &mut self.text_backend,
                        );
                        if !result.is_consumed() {
                            result = EventResult::Consumed;
                        }
                    }
                }
                // Scrolling is the default action of a navigation key, so it
                // goes last, after widgets and shortcuts have both passed.
                if !result.is_consumed()
                    && let Some(raw) = keyboard
                {
                    result = self
                        .app
                        .dispatch_keyboard_scroll(&raw, &mut self.text_backend);
                }
                self.refresh_platform_output();
                vec![result]
            }
            RuntimeEvent::RedrawRequested => Vec::new(),
            RuntimeEvent::Exit => {
                self.control_flow = ControlFlow::Exit;
                Vec::new()
            }
        }
    }

    pub fn frame(&mut self) -> Result<FrameReport, AppRenderError<B::Error>> {
        self.frame_at(Instant::now())
    }

    /// [`Self::frame`] with the frame's instant supplied by the caller.
    ///
    /// The loop reads the clock in exactly one place, which is here, so a test
    /// or a benchmark can drive whole sequences of frames at chosen instants
    /// and get the same animation out of them every run.
    pub fn frame_at(&mut self, now: Instant) -> Result<FrameReport, AppRenderError<B::Error>> {
        self.app.drain_async_messages();
        // Sampled once and published before anything is stepped: two
        // animations in one frame have to agree on when that frame is.
        //
        // Off screen the frame still happens -- something invalidated the
        // window and the picture has to be right the instant it is revealed --
        // but no time passes in it, so nothing moves through frames nobody saw.
        let frame = if self.app.window_visible() {
            self.frame_clock.tick(now)
        } else {
            self.frame_clock.hold()
        };
        self.app.begin_frame(frame);
        self.app.tick_style_animations(frame.delta());
        // Before `is_dirty`, so whatever these two set in motion is part of
        // this frame rather than of the one after it.
        self.app.tick_animating_canvases();
        self.app.tick_tickers(frame);
        let should_render = self.app.is_dirty();
        if should_render {
            self.app.render(&mut self.backend, &mut self.text_backend)?;
        }
        if !self.app.is_dirty() {
            // Nothing asked for another frame, so the loop is about to wait on
            // an event and the gap until the next frame is unbounded. Stopping
            // the clock here is what makes that gap cost nothing: whenever the
            // next frame comes, it starts a fresh delta instead of replaying
            // the idle into whatever animation the event started.
            self.frame_clock.suspend();
        }
        self.refresh_platform_output();
        Ok(FrameReport {
            rendered: should_render,
            event_results: Vec::new(),
        })
    }

    pub fn text_measure(&self) -> &T {
        self.text_backend.backend()
    }

    pub fn text_measure_mut(&mut self) -> &mut T {
        self.text_backend.backend_mut()
    }

    pub fn tick(
        &mut self,
        event_source: &mut impl EventSource<Event = RuntimeEvent>,
    ) -> Result<FrameReport, AppRenderError<B::Error>> {
        let mut event_results = Vec::new();
        while let Some(event) = event_source.poll_event() {
            event_results.extend(self.handle_event(event));
            if self.control_flow == ControlFlow::Exit {
                return Ok(FrameReport {
                    rendered: false,
                    event_results,
                });
            }
        }

        let mut report = self.frame()?;
        report.event_results = event_results;
        Ok(report)
    }

    pub fn run_until_idle(
        &mut self,
        event_source: &mut impl EventSource<Event = RuntimeEvent>,
    ) -> Result<FrameReport, AppRenderError<B::Error>> {
        let mut merged = FrameReport {
            rendered: false,
            event_results: Vec::new(),
        };

        loop {
            let report = self.tick(event_source)?;
            merged.rendered |= report.rendered;
            merged.event_results.extend(report.event_results);
            if self.control_flow == ControlFlow::Exit || !self.app.is_dirty() {
                break;
            }
        }

        Ok(merged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::element::ElementDesc;
    use crate::render::MockRenderBackend;
    use crate::state::HookContext;
    use crate::text::testing::ZeroTextBackend;
    use crate::widgets::container;
    use std::cell::{Cell, RefCell};
    use std::time::Duration;

    fn body() -> ElementDesc {
        container()
            .style(xui_interface::Style::new().width(40.0).height(20.0))
            .into_element_desc(Vec::new())
    }

    fn root(_cx: &mut HookContext<'_>) -> ElementDesc {
        body()
    }

    thread_local! {
        static TICKS: RefCell<Vec<FrameTime>> = const { RefCell::new(Vec::new()) };
        static RENDERS: Cell<usize> = const { Cell::new(0) };
        static TICKER: RefCell<Option<crate::ticker::Ticker>> = const { RefCell::new(None) };
    }

    fn ticking_root(cx: &mut HookContext<'_>) -> ElementDesc {
        RENDERS.with(|renders| renders.set(renders.get() + 1));
        let ticker = cx.use_ticker(|frame| {
            TICKS.with(|ticks| ticks.borrow_mut().push(frame));
            crate::ticker::Tick::Continue
        });
        TICKER.with(|slot| *slot.borrow_mut() = Some(ticker));
        body()
    }

    fn reset_ticking_root() {
        TICKS.with(|ticks| ticks.borrow_mut().clear());
        RENDERS.with(|renders| renders.set(0));
        TICKER.with(|slot| *slot.borrow_mut() = None);
    }

    fn ticks() -> Vec<FrameTime> {
        TICKS.with(|ticks| ticks.borrow().clone())
    }

    fn runtime() -> GuiRuntime<MockRenderBackend, ZeroTextBackend> {
        runtime_with(root)
    }

    fn runtime_with(
        component: crate::app::ComponentFn,
    ) -> GuiRuntime<MockRenderBackend, ZeroTextBackend> {
        let mut runtime = GuiRuntime::new(
            App::new(component),
            MockRenderBackend::default(),
            ZeroTextBackend,
        );
        runtime.handle_event(RuntimeEvent::Resize(Size::new(400.0, 300.0)));
        runtime
    }

    mod keyboard_scroll {
        use super::*;
        use crate::event_system::callbacks::EventProps;
        use xui_interface::events::{
            EventResult, KeyState, Modifiers, NamedKey, PhysicalKey, RawKeyboard, ScrollSource,
        };
        use xui_interface::{CommandId, Shortcut};

        thread_local! {
            static SCROLLS: RefCell<Vec<(ScrollSource, f32)>> = const { RefCell::new(Vec::new()) };
        }

        /// A 200x100 vertical scroller over a focusable 400px row that claims
        /// End through a shortcut, the way a tab list claims its arrows.
        fn root(_cx: &mut HookContext<'_>) -> ElementDesc {
            container()
                .style(
                    xui_interface::Style::new()
                        .width(200.0)
                        .height(100.0)
                        .scroll_vertical(),
                )
                .on_scroll(|event, _| {
                    if let Some(after) = event.offset_after {
                        SCROLLS.with(|scrolls| scrolls.borrow_mut().push((event.source, after.y)));
                    }
                })
                .into_element_desc(vec![
                    container()
                        .style(xui_interface::Style::new().width(200.0).height(400.0))
                        .tab_index(0)
                        .shortcut(Shortcut::named(NamedKey::End), CommandId("test.end"))
                        .on_command(|_, _| EventResult::Consumed)
                        .into_element_desc(Vec::new()),
                ])
        }

        fn key(named: NamedKey, shift: bool) -> RuntimeEvent {
            RuntimeEvent::Input(RawEvent::Keyboard(RawKeyboard {
                physical_key: PhysicalKey::Unidentified,
                named_key: Some(named),
                state: KeyState::Down,
                text: None,
                modifiers: Modifiers {
                    shift,
                    ..Modifiers::default()
                },
                timestamp: Instant::now(),
                is_repeat: false,
            }))
        }

        fn offsets() -> Vec<f32> {
            SCROLLS.with(|scrolls| {
                scrolls
                    .borrow()
                    .iter()
                    .map(|(source, y)| {
                        assert_eq!(*source, ScrollSource::Keyboard);
                        *y
                    })
                    .collect()
            })
        }

        #[test]
        fn unclaimed_navigation_keys_scroll_the_focused_nodes_container() {
            let mut runtime = runtime_with(root);
            runtime.frame().unwrap();
            runtime.handle_event(key(NamedKey::Tab, false));
            assert!(runtime.app().ui_runtime().focused_node().is_some());

            for (named, shift) in [
                (NamedKey::PageDown, false),
                (NamedKey::ArrowDown, false),
                (NamedKey::Home, false),
                (NamedKey::Space, false),
                (NamedKey::Space, true),
            ] {
                assert_eq!(
                    runtime.handle_event(key(named, shift)),
                    vec![EventResult::Consumed],
                    "{named:?} with shift={shift} did not scroll"
                );
            }

            // Page steps are 87.5% of the 100px viewport; a line is 40px.
            assert_eq!(offsets(), vec![87.5, 127.5, 0.0, 87.5, 0.0]);
        }

        #[test]
        fn a_key_claimed_by_a_shortcut_does_not_scroll() {
            let mut runtime = runtime_with(root);
            runtime.frame().unwrap();
            runtime.handle_event(key(NamedKey::Tab, false));

            assert_eq!(
                runtime.handle_event(key(NamedKey::End, false)),
                vec![EventResult::Consumed]
            );
            assert!(offsets().is_empty(), "End reached the shortcut and still scrolled");
        }
    }

    /// Keeps `is_dirty` true the way an animating canvas does, so the loop
    /// stays continuous without a real painter to drive.
    fn keep_animating(runtime: &mut GuiRuntime<MockRenderBackend, ZeroTextBackend>) {
        let root = runtime.app().ui_runtime().root();
        runtime
            .app_mut()
            .ui_runtime_mut()
            .canvases_wanting_repaint
            .insert(root, ());
    }

    #[test]
    fn the_frame_time_reaches_the_app_before_the_frame_runs() {
        let start = Instant::now();
        let mut runtime = runtime();
        keep_animating(&mut runtime);

        runtime.frame_at(start).unwrap();
        assert_eq!(runtime.app().frame_time().index(), 1);
        assert_eq!(runtime.app().frame_time().delta(), Duration::ZERO);

        runtime
            .frame_at(start + Duration::from_millis(16))
            .unwrap();

        let frame = runtime.app().frame_time();
        assert_eq!(frame.index(), 2);
        assert_eq!(frame.delta(), Duration::from_millis(16));
        assert_eq!(frame.timestamp(), Duration::from_millis(16));
        assert_eq!(
            frame,
            runtime.frame_time(),
            "the runtime and the app read the same frame"
        );
    }

    #[test]
    fn an_idle_stretch_does_not_become_a_frame_step() {
        let start = Instant::now();
        let mut runtime = runtime();

        // Mount, then keep going until nothing is dirty any more.
        for step in 0..4 {
            runtime
                .frame_at(start + Duration::from_millis(16 * step))
                .unwrap();
        }
        assert!(
            runtime.frame_clock().is_suspended(),
            "an idle app stops the clock"
        );
        let idle_at = runtime.frame_time().timestamp();

        // A minute later, an event wakes the loop back up.
        runtime
            .frame_at(start + Duration::from_secs(60))
            .unwrap();

        let frame = runtime.frame_time();
        assert_eq!(
            frame.delta(),
            Duration::ZERO,
            "the wait is not time an animation should advance through"
        );
        assert_eq!(frame.timestamp(), idle_at);
    }

    #[test]
    fn a_hidden_window_stops_asking_for_frames() {
        let start = Instant::now();
        let mut runtime = runtime();
        // Settle the mount first, so the only thing left dirty is animation.
        for step in 0..4 {
            runtime
                .frame_at(start + Duration::from_millis(16 * step))
                .unwrap();
        }
        assert!(!runtime.app().is_dirty());

        keep_animating(&mut runtime);
        assert!(runtime.app().is_dirty());

        runtime.set_window_visible(false);
        assert!(
            !runtime.app().is_dirty(),
            "nothing anyone can see is animating, so nothing should schedule a frame"
        );

        runtime.set_window_visible(true);
        assert!(
            runtime.app().is_dirty(),
            "the request outlives the pause, so revealing resumes it"
        );
    }

    #[test]
    fn time_does_not_pass_while_the_window_is_hidden() {
        let start = Instant::now();
        let mut runtime = runtime();
        keep_animating(&mut runtime);

        runtime.frame_at(start).unwrap();
        runtime.frame_at(start + Duration::from_millis(16)).unwrap();
        let before = runtime.frame_time();
        assert_eq!(before.timestamp(), Duration::from_millis(16));

        runtime.set_window_visible(false);
        // Something invalidated the window while it was hidden -- a resize, a
        // state change. It still has to paint, and it still must not move.
        runtime.app_mut().mark_needs_rebuild();
        runtime
            .frame_at(start + Duration::from_secs(30))
            .unwrap();
        let held = runtime.frame_time();
        assert_eq!(held.delta(), Duration::ZERO);
        assert_eq!(held.timestamp(), before.timestamp());
        assert_eq!(held.index(), before.index() + 1, "the frame still happened");

        runtime.set_window_visible(true);
        runtime
            .frame_at(start + Duration::from_secs(60))
            .unwrap();
        let resumed = runtime.frame_time();
        assert_eq!(
            resumed.delta(),
            Duration::ZERO,
            "the minute off screen is not a step any animation takes"
        );
        assert_eq!(resumed.timestamp(), before.timestamp());

        runtime
            .frame_at(start + Duration::from_secs(60) + Duration::from_millis(16))
            .unwrap();
        assert_eq!(
            runtime.frame_time().timestamp(),
            Duration::from_millis(32),
            "and from there it carries on exactly where it left off"
        );
    }

    #[test]
    fn a_ticker_runs_once_a_frame_and_does_not_rebuild_its_component() {
        reset_ticking_root();
        let start = Instant::now();
        let mut runtime = runtime_with(ticking_root);

        let frames: Vec<_> = (0..4)
            .map(|step| start + Duration::from_millis(16 * step))
            .collect();
        for at in &frames {
            runtime.frame_at(*at).unwrap();
        }

        let ticks = ticks();
        assert_eq!(
            ticks.len(),
            3,
            "installed during the first frame's render, so it runs on the three after it"
        );
        assert_eq!(
            ticks.iter().map(|tick| tick.index()).collect::<Vec<_>>(),
            vec![2, 3, 4],
            "once per frame, with that frame's own time"
        );
        assert!(
            ticks
                .iter()
                .all(|tick| tick.delta() == Duration::from_millis(16))
        );
        assert_eq!(
            RENDERS.with(Cell::get),
            1,
            "a ticking frame is not a reconciling frame -- that is the whole point"
        );
    }

    #[test]
    fn a_running_ticker_is_what_keeps_the_loop_awake() {
        reset_ticking_root();
        let start = Instant::now();
        let mut runtime = runtime_with(ticking_root);
        runtime.frame_at(start).unwrap();

        assert!(
            runtime.app().is_dirty(),
            "a ticker asks for the next frame the way an animating canvas does"
        );

        let ticker = TICKER.with(|slot| slot.borrow().clone()).unwrap();
        ticker.stop();
        runtime
            .frame_at(start + Duration::from_millis(16))
            .unwrap();
        assert!(!runtime.app().is_dirty(), "and stopping it lets the loop idle");

        let before = ticks().len();
        runtime
            .frame_at(start + Duration::from_millis(32))
            .unwrap();
        assert_eq!(ticks().len(), before, "a stopped ticker does not run");

        ticker.start();
        assert!(runtime.app().is_dirty());
        runtime
            .frame_at(start + Duration::from_millis(48))
            .unwrap();
        assert_eq!(ticks().len(), before + 1);
    }

    #[test]
    fn a_hidden_window_does_not_tick() {
        reset_ticking_root();
        let start = Instant::now();
        let mut runtime = runtime_with(ticking_root);
        runtime.frame_at(start).unwrap();
        runtime
            .frame_at(start + Duration::from_millis(16))
            .unwrap();
        let before = ticks().len();
        assert_eq!(before, 1);

        runtime.set_window_visible(false);
        // Hidden, but something invalidated the window, so a frame still runs.
        runtime.app_mut().mark_needs_rebuild();
        runtime
            .frame_at(start + Duration::from_secs(10))
            .unwrap();
        assert_eq!(
            ticks().len(),
            before,
            "a frame nobody sees must not advance the simulation"
        );

        runtime.set_window_visible(true);
        runtime
            .frame_at(start + Duration::from_secs(20))
            .unwrap();
        let resumed = ticks();
        assert_eq!(resumed.len(), before + 1);
        assert_eq!(
            resumed.last().unwrap().delta(),
            Duration::ZERO,
            "and the frame it comes back on is a fresh step, not a ten second one"
        );
    }

    #[test]
    fn a_stalled_frame_is_clamped_rather_than_skipped() {
        let start = Instant::now();
        let mut runtime = runtime();
        keep_animating(&mut runtime);

        runtime.frame_at(start).unwrap();
        runtime
            .frame_at(start + Duration::from_secs(2))
            .unwrap();

        assert_eq!(
            runtime.frame_time().delta(),
            FrameClock::DEFAULT_MAX_DELTA,
            "a two second stall must not fast-forward every animation past its end"
        );
    }
}
