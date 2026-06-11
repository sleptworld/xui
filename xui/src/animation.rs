use std::time::Duration;

use xui_animation::{Animatable, Timeline};
use xui_interface::{AnimationTransition, EventTrigger, Style};

#[derive(Debug, Clone, PartialEq)]
pub struct ActiveStyleAnimation {
    pub trigger: EventTrigger,
    pub from_style: Style,
    pub to_style: Style,
    pub timeline: Timeline,
    completed: bool,
}

impl ActiveStyleAnimation {
    pub fn new(
        trigger: EventTrigger,
        from_style: Style,
        to_style: Style,
        transition: AnimationTransition,
    ) -> Self {
        Self {
            trigger,
            from_style,
            to_style,
            timeline: Timeline::new(transition.into()),
            completed: false,
        }
    }

    pub fn sample(&self) -> Style {
        let progress = self.timeline.progress().eased;
        Style::interpolate(&self.from_style, &self.to_style, progress)
    }

    pub fn tick(&mut self, delta: Duration) -> bool {
        if self.completed {
            return false;
        }

        let progress = self.timeline.tick(delta);
        self.completed = progress.completed;
        true
    }

    pub fn is_completed(&self) -> bool {
        self.completed
    }

    pub fn is_running(&self) -> bool {
        !self.completed
    }
}
