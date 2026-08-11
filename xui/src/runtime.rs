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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ElementDesc;
    use crate::render::MockRenderBackend;
    use crate::state::HookContext;
    use crate::text::testing::ZeroTextBackend;
    use crate::widgets::{container, text_input};
    use std::cell::Cell;
    use std::time::Instant;
    use xui_interface::{
        CommandId, FocusReason, KeyState, Modifiers, PhysicalKey, RawKeyboard, RawWindowEvent,
        Shortcut, Size, Style,
    };

    thread_local! { static COMMAND_RECEIVED: Cell<bool> = const { Cell::new(false) }; }

    fn command_root(_cx: &mut HookContext<'_>) -> ElementDesc {
        container()
            .on_command(|event, _| {
                if event.command == CommandId("save") {
                    COMMAND_RECEIVED.set(true);
                }
                EventResult::Consumed
            })
            .into_element_desc(Vec::new())
    }

    #[test]
    fn runtime_shortcut_dispatches_command_to_app_root() {
        COMMAND_RECEIVED.set(false);
        let app = App::new(command_root);
        let mut runtime = GuiRuntime::new(app, MockRenderBackend::default(), ZeroTextBackend);
        let shortcut = Shortcut::physical(PhysicalKey::KeyS);
        runtime
            .shortcuts_mut()
            .register(shortcut, CommandId("save"));
        let raw = RawKeyboard {
            physical_key: PhysicalKey::KeyS,
            named_key: None,
            state: KeyState::Down,
            text: None,
            modifiers: Modifiers::default(),
            timestamp: Instant::now(),
            is_repeat: false,
        };
        assert_eq!(
            runtime.handle_event(RuntimeEvent::Input(RawEvent::Keyboard(raw))),
            vec![EventResult::Consumed]
        );
        assert!(COMMAND_RECEIVED.get());
    }

    fn text_input_root(_cx: &mut HookContext<'_>) -> ElementDesc {
        text_input()
            .style(Style::new().width(120.0).height(30.0))
            .into_element_desc()
    }

    #[test]
    fn focused_text_input_publishes_platform_ime_session() {
        let app = App::new(text_input_root);
        let mut runtime = GuiRuntime::new(app, MockRenderBackend::default(), ZeroTextBackend);
        runtime.handle_event(RuntimeEvent::Resize(Size::new(200.0, 100.0)));
        runtime.frame().unwrap();

        let input = runtime.app().arena().children(runtime.app().arena().root())[0];
        runtime
            .app_mut()
            .arena_mut()
            .focus_manager_mut()
            .request_focus(Some(input), FocusReason::Programmatic);
        runtime.handle_event(RuntimeEvent::Input(RawEvent::WindowFocus(RawWindowEvent {
            timestamp: Instant::now(),
            modifiers: Modifiers::default(),
        })));

        let session = runtime
            .platform_output()
            .text_input
            .as_ref()
            .expect("focused text input should enable the platform IME session");
        assert!(session.cursor_area.width >= 1.0);
        assert!(session.cursor_area.height >= 1.0);
        assert!(!session.multiline);
    }
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
    last_animation_tick: Option<Instant>,
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
            last_animation_tick: None,
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

    pub fn control_flow(&self) -> ControlFlow {
        self.control_flow
    }

    pub fn set_control_flow(&mut self, control_flow: ControlFlow) {
        self.control_flow = control_flow;
    }

    pub fn platform_output(&self) -> &PlatformOutput {
        &self.platform_output
    }

    fn refresh_platform_output(&mut self) {
        let arena = self.app.arena();
        let text_input = arena.focus_manager().focused().and_then(|id| {
            let node = arena.node(id)?;
            let rect = arena.visual_layout(id)?;
            let handle = self.text_backend.active_slot(id, TextLayoutSlot::PRIMARY)?;
            let layout = self.text_backend.query(handle)?;
            node.widget.platform_text_input_session(rect, layout)
        });
        self.platform_output = PlatformOutput { text_input };
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
                if !result.is_consumed() {
                    if let Some(raw) = keyboard {
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
        self.app.drain_async_messages();
        self.tick_style_animations();
        let should_render = self.app.is_dirty();
        if should_render {
            self.app.render(&mut self.backend, &mut self.text_backend)?;
        }
        self.refresh_platform_output();
        Ok(FrameReport {
            rendered: should_render,
            event_results: Vec::new(),
        })
    }

    fn tick_style_animations(&mut self) {
        if !self.app.has_running_style_animations() {
            self.last_animation_tick = None;
            return;
        }

        let now = Instant::now();
        let delta = self
            .last_animation_tick
            .map(|last| now.saturating_duration_since(last))
            .unwrap_or_default();
        self.last_animation_tick = Some(now);
        self.app.tick_style_animations(delta);
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
