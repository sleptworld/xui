use std::collections::VecDeque;

use crate::app::App;
use crate::core::Size;
use crate::event::{Event, EventResult};
use crate::render::RenderBackend;
pub use xui_interface::EventSource;
use xui_interface::TextMeasurer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlFlow {
    Poll,
    Wait,
    Exit,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeEvent {
    Resize(Size),
    Input(Event),
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

pub struct GuiRuntime<B: RenderBackend<T>, T: TextMeasurer> {
    app: App,
    backend: B,
    control_flow: ControlFlow,
    text_measure: T,
}

impl<B: RenderBackend<T>, T: TextMeasurer> GuiRuntime<B, T> {
    pub fn new(app: App, backend: B, text_measure: T) -> Self {
        Self {
            app,
            backend,
            control_flow: ControlFlow::Poll,
            text_measure,
        }
    }

    pub fn app(&self) -> &App {
        &self.app
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

    pub fn handle_event(&mut self, event: RuntimeEvent) -> Vec<EventResult> {
        match event {
            RuntimeEvent::Resize(size) => {
                self.app.resize(size);
                Vec::new()
            }
            RuntimeEvent::Input(event) => {
                vec![self.app.dispatch_event(event, &mut self.text_measure)]
            }
            RuntimeEvent::RedrawRequested => Vec::new(),
            RuntimeEvent::Exit => {
                self.control_flow = ControlFlow::Exit;
                Vec::new()
            }
        }
    }

    pub fn frame(&mut self) -> Result<FrameReport, B::Error> {
        let should_render = self.app.is_dirty();
        if should_render {
            self.app.render(&mut self.backend, &mut self.text_measure)?;
        }
        Ok(FrameReport {
            rendered: should_render,
            event_results: Vec::new(),
        })
    }

    pub fn tick(
        &mut self,
        event_source: &mut impl EventSource<Event = RuntimeEvent>,
    ) -> Result<FrameReport, B::Error> {
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
    ) -> Result<FrameReport, B::Error> {
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
