use std::time::Duration;
use xui_animation::{Animatable, Timeline};
use xui_interface::{
    AnimationTransition, Color, ColorStyle, ComputedPaintStyle, ComputedTextStyle, EventTrigger,
    Point, TextStyle,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ActiveAnimation<A: Animatable> {
    pub trigger: EventTrigger,
    pub from_style: A,
    pub to_style: A,
    pub timeline: Timeline,
    completed: bool,
}

impl<A: Animatable> ActiveAnimation<A> {
    pub fn new(
        trigger: EventTrigger,
        from_style: A,
        to_style: A,
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

    pub fn sample(&self) -> A {
        let progress = self.timeline.progress().eased;
        A::interpolate(&self.from_style, &self.to_style, progress)
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

#[derive(Clone, Animatable, Default)]
pub struct AnimableStyle {
    pub text: AnimableTextStyle,
    pub paint: AnimablePaintStyle,
}

#[derive(Animatable, Clone, Copy, Default)]
pub struct AnimableTextStyle {
    pub color: ColorStyle,
    pub font_size: f32,
}

#[derive(Animatable, Clone, Copy, Default)]
pub struct AnimablePaintStyle {
    pub background: ColorStyle,
    pub border_color: ColorStyle,
    pub border_width: f32,
    pub border_radius: f32,
}

#[derive(Animatable, Clone, Copy, Default)]
pub struct AnimableShadowStyle {
    pub color: Color,
    pub offset: Point,
    pub blur: f32,
    pub spread: f32,
}

#[derive(Animatable, Clone, Copy, Default)]
pub struct AnimableStrokeStyle {
    pub color: ColorStyle,
    pub width: f32,
}
