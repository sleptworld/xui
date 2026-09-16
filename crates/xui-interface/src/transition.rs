use easer::functions::{Cubic, Easing as EaserFunction, Linear, Quad, Quart, Quint, Sine};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Hash, Default)]
pub enum Easing {
    #[default]
    Linear,
    QuadIn,
    QuadOut,
    QuadInOut,
    CubicIn,
    CubicOut,
    CubicInOut,
    QuartIn,
    QuartOut,
    QuartInOut,
    QuintIn,
    QuintOut,
    QuintInOut,
    SineIn,
    SineOut,
    SineInOut,
}

impl Easing {
    pub fn sample(self, progress: f32) -> f32 {
        let progress = progress.clamp(0.0, 1.0);
        let eased = match self {
            Self::Linear => Linear::ease_in(progress, 0.0, 1.0, 1.0),
            Self::QuadIn => Quad::ease_in(progress, 0.0, 1.0, 1.0),
            Self::QuadOut => Quad::ease_out(progress, 0.0, 1.0, 1.0),
            Self::QuadInOut => Quad::ease_in_out(progress, 0.0, 1.0, 1.0),
            Self::CubicIn => Cubic::ease_in(progress, 0.0, 1.0, 1.0),
            Self::CubicOut => Cubic::ease_out(progress, 0.0, 1.0, 1.0),
            Self::CubicInOut => Cubic::ease_in_out(progress, 0.0, 1.0, 1.0),
            Self::QuartIn => Quart::ease_in(progress, 0.0, 1.0, 1.0),
            Self::QuartOut => Quart::ease_out(progress, 0.0, 1.0, 1.0),
            Self::QuartInOut => Quart::ease_in_out(progress, 0.0, 1.0, 1.0),
            Self::QuintIn => Quint::ease_in(progress, 0.0, 1.0, 1.0),
            Self::QuintOut => Quint::ease_out(progress, 0.0, 1.0, 1.0),
            Self::QuintInOut => Quint::ease_in_out(progress, 0.0, 1.0, 1.0),
            Self::SineIn => Sine::ease_in(progress, 0.0, 1.0, 1.0),
            Self::SineOut => Sine::ease_out(progress, 0.0, 1.0, 1.0),
            Self::SineInOut => Sine::ease_in_out(progress, 0.0, 1.0, 1.0),
        };
        eased.clamp(0.0, 1.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Hash)]
pub struct Transition {
    pub duration: Duration,
    pub delay: Duration,
    pub easing: Easing,
}

impl Transition {
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            delay: Duration::ZERO,
            easing: Easing::default(),
        }
    }

    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
    pub fn ease(mut self, easing: Easing) -> Self {
        self.easing = easing;
        self
    }

    pub fn progress_at(self, elapsed: Duration) -> AnimationProgress {
        let raw = if elapsed <= self.delay {
            0.0
        } else if self.duration.is_zero() {
            1.0
        } else {
            elapsed.saturating_sub(self.delay).as_secs_f32() / self.duration.as_secs_f32()
        }
        .clamp(0.0, 1.0);
        AnimationProgress {
            raw,
            eased: self.easing.sample(raw),
            completed: elapsed >= self.delay.saturating_add(self.duration),
        }
    }
}

impl Default for Transition {
    fn default() -> Self {
        Self::new(Duration::ZERO)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationProgress {
    pub raw: f32,
    pub eased: f32,
    pub completed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_respects_delay_and_completion() {
        let transition = Transition::new(Duration::from_millis(100))
            .delay(Duration::from_millis(50))
            .ease(Easing::Linear);
        assert_eq!(transition.progress_at(Duration::from_millis(25)).raw, 0.0);
        assert_eq!(transition.progress_at(Duration::from_millis(100)).raw, 0.5);
        assert!(transition.progress_at(Duration::from_millis(150)).completed);
    }
}
