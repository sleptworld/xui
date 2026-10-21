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

/// How many times an [`Animation`] plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Iterations {
    Count(u32),
    /// Until the style stops asking for it, or the node goes away.
    Infinite,
}

/// Which way each iteration of an [`Animation`] runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AnimationDirection {
    /// `from` to `to`, every time.
    #[default]
    Normal,
    /// `to` to `from`, every time.
    Reverse,
    /// `from` to `to`, then back, and so on.
    Alternate,
    /// `to` to `from`, then back, and so on.
    AlternateReverse,
}

/// A style that moves on its own, between two keyframes, for a set number of
/// iterations or forever: a spinner turning, a skeleton pulsing.
///
/// Where a [`Transition`] animates the change from one target to the next and
/// stops, an animation has nothing to wait for. It starts when the node mounts
/// with it (or when its style starts asking for it). Changing the animation
/// restarts it; re-rendering with the same one does not. A finite animation
/// that finishes leaves the node at its style, not at the last keyframe --
/// give the style the end values to keep them.
///
/// This is the animation's timing only. Its two keyframes are layers of the
/// node's own style, declared with the node — so a keyframe can name only
/// properties the node has — and kept per style capability, like state
/// rules.
///
/// It is a layer on top of the node's style, per property: each keyframe is
/// what the node would otherwise show with the keyframe layered on. A
/// property neither keyframe names shows through unchanged, transition
/// included, so a spinner can turn while its color transitions. On a property
/// both name, the animation wins while it plays, and a transition started
/// meanwhile runs underneath from where the node's style was, not from the
/// animation's frame.
///
/// Only a transform is cheap to animate every frame -- the renderer moves the
/// node's picture without repainting it. Anything else repaints the node each
/// frame, and anything that changes its size lays it out again.
#[derive(Debug, Clone, Copy, PartialEq, Hash)]
pub struct Animation {
    /// One iteration.
    pub duration: Duration,
    /// Before the first iteration, during which the node shows `from` (or `to`,
    /// if the first iteration runs backwards).
    pub delay: Duration,
    /// Applied within each iteration, in the direction it plays.
    pub easing: Easing,
    pub iterations: Iterations,
    pub direction: AnimationDirection,
}

impl Animation {
    /// Plays once, forwards, linearly, with no delay.
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            delay: Duration::ZERO,
            easing: Easing::Linear,
            iterations: Iterations::Count(1),
            direction: AnimationDirection::Normal,
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

    pub fn iterations(mut self, iterations: Iterations) -> Self {
        self.iterations = iterations;
        self
    }

    /// Shorthand for `iterations(Iterations::Infinite)`.
    pub fn infinite(self) -> Self {
        self.iterations(Iterations::Infinite)
    }

    pub fn direction(mut self, direction: AnimationDirection) -> Self {
        self.direction = direction;
        self
    }

    /// Where the animation is after `elapsed` of playing, as progress from
    /// `from` (0) to `to` (1).
    ///
    /// `completed` is true once a finite animation has played every
    /// iteration; an infinite one never completes. One with no iterations, or
    /// an infinite one with no duration, is complete from the start rather
    /// than looping without moving.
    pub fn progress_at(&self, elapsed: Duration) -> AnimationProgress {
        let total = match self.iterations {
            Iterations::Count(count) => Some(count),
            Iterations::Infinite => None,
        };
        if total == Some(0) || (self.duration.is_zero() && total.is_none()) {
            return self.sample(0, 1.0, true);
        }
        let Some(playing) = elapsed.checked_sub(self.delay) else {
            return self.sample(0, 0.0, false);
        };
        let (iteration, fraction) = if self.duration.is_zero() {
            (u64::MAX, 0.0)
        } else {
            let duration = self.duration.as_nanos();
            let playing = playing.as_nanos();
            let iteration = u64::try_from(playing / duration).unwrap_or(u64::MAX);
            let fraction = (playing % duration) as f64 / duration as f64;
            (iteration, fraction as f32)
        };
        match total {
            Some(count) if iteration >= u64::from(count) => {
                self.sample(u64::from(count) - 1, 1.0, true)
            }
            _ => self.sample(iteration, fraction, false),
        }
    }

    /// `fraction` of the way through `iteration`, in the direction it plays.
    fn sample(&self, iteration: u64, fraction: f32, completed: bool) -> AnimationProgress {
        let even = iteration % 2 == 0;
        let forwards = match self.direction {
            AnimationDirection::Normal => true,
            AnimationDirection::Reverse => false,
            AnimationDirection::Alternate => even,
            AnimationDirection::AlternateReverse => !even,
        };
        let raw = if forwards { fraction } else { 1.0 - fraction };
        let eased = if forwards {
            self.easing.sample(fraction)
        } else {
            1.0 - self.easing.sample(fraction)
        };
        AnimationProgress {
            raw,
            eased,
            completed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spin() -> Animation {
        Animation::new(Duration::from_millis(100))
    }

    fn at(animation: &Animation, millis: u64) -> AnimationProgress {
        animation.progress_at(Duration::from_millis(millis))
    }

    #[test]
    fn an_infinite_animation_wraps_and_never_completes() {
        let animation = spin().infinite();
        assert_eq!(at(&animation, 25).raw, 0.25);
        assert_eq!(at(&animation, 125).raw, 0.25);
        assert!((at(&animation, 100_075).raw - 0.75).abs() < 1e-6);
        assert!(!at(&animation, 100_075).completed);
    }

    #[test]
    fn a_finite_animation_ends_on_its_last_iteration() {
        let animation = spin()
            .iterations(Iterations::Count(2))
            .delay(Duration::from_millis(50));
        assert_eq!(
            at(&animation, 0).raw,
            0.0,
            "the delay shows the first keyframe"
        );
        assert!(!at(&animation, 249).completed);
        let done = at(&animation, 250);
        assert!(done.completed);
        assert_eq!(done.raw, 1.0);
    }

    #[test]
    fn directions_choose_which_way_each_iteration_runs() {
        let alternate = spin().infinite().direction(AnimationDirection::Alternate);
        assert_eq!(at(&alternate, 25).raw, 0.25);
        assert_eq!(at(&alternate, 125).raw, 0.75);
        let reverse = spin().infinite().direction(AnimationDirection::Reverse);
        assert_eq!(at(&reverse, 25).raw, 0.75);
        let ended = spin()
            .iterations(Iterations::Count(2))
            .direction(AnimationDirection::Alternate);
        assert_eq!(
            at(&ended, 500).raw,
            0.0,
            "an even count of alternations ends at from"
        );
    }

    #[test]
    fn easing_runs_in_the_direction_of_play() {
        let animation = spin()
            .infinite()
            .ease(Easing::QuadIn)
            .direction(AnimationDirection::Alternate);
        let forwards = at(&animation, 25).eased;
        let backwards = at(&animation, 125).eased;
        assert!((forwards - 0.0625).abs() < 1e-6);
        assert!((backwards - (1.0 - 0.0625)).abs() < 1e-6);
    }

    #[test]
    fn an_animation_with_nothing_to_play_is_complete_at_once() {
        assert!(at(&spin().iterations(Iterations::Count(0)), 0).completed);
        let still = Animation::new(Duration::ZERO).infinite();
        assert!(at(&still, 0).completed);
    }

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
