//! A node's cached layout results, in place of Taffy's `Cache`.
//!
//! Taffy's cache gives each *kind* of sizing query one slot -- which of the
//! two dimensions is known, and whether the rest is min-content or not -- and
//! the key stored in a slot includes the parent's width. A flex container asks
//! the same kind of question of a child more than once per pass with
//! different parent sizes (its intrinsic size, then its line), so the two
//! answers evict each other: every pass recomputes every node, and changing
//! one leaf lays the whole tree out again.
//!
//! This cache answers exactly what Taffy's would -- a measurement matches on
//! the known dimensions, the available space and the parent's width; a final
//! layout on the whole input -- but keeps several measurements per node
//! instead of one per kind.

use taffy as tf;

/// How many measurements a node keeps. A pass asks a node at most a handful
/// of distinct questions; when they do not fit, the oldest is replaced.
const MEASUREMENTS: usize = 8;

/// Quiet-NaN payloads that encode what is not a number. A computed NaN is
/// the canonical one, payload 0, so these never collide with a value.
const NONE: u32 = 0x7fc0_0001;
const MIN_CONTENT: u32 = 0x7fc0_0002;
const MAX_CONTENT: u32 = 0x7fc0_0003;

fn option(value: Option<f32>) -> u32 {
    value.map_or(NONE, f32::to_bits)
}

fn space(value: tf::AvailableSpace) -> u32 {
    match value {
        tf::AvailableSpace::Definite(value) => value.to_bits(),
        tf::AvailableSpace::MinContent => MIN_CONTENT,
        tf::AvailableSpace::MaxContent => MAX_CONTENT,
    }
}

/// What a measurement depends on, bit for bit.
#[derive(Clone, Copy, PartialEq, Eq)]
struct MeasureKey {
    known: [u32; 2],
    available: [u32; 2],
    parent_width: u32,
}

impl MeasureKey {
    fn of(input: &tf::LayoutInput) -> Self {
        Self {
            known: [
                option(input.known_dimensions.width),
                option(input.known_dimensions.height),
            ],
            available: [
                space(input.available_space.width),
                space(input.available_space.height),
            ],
            parent_width: option(input.parent_size.width),
        }
    }
}

/// What a final layout depends on: everything a measurement does, the
/// parent's height, and the axis asked for.
#[derive(Clone, Copy, PartialEq, Eq)]
struct FinalKey {
    measure: MeasureKey,
    parent_height: u32,
    axis: tf::RequestedAxis,
}

impl FinalKey {
    fn of(input: &tf::LayoutInput) -> Self {
        Self {
            measure: MeasureKey::of(input),
            parent_height: option(input.parent_size.height),
            axis: input.axis,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct LayoutCache {
    final_layout: Option<(FinalKey, tf::LayoutOutput)>,
    measurements: [Option<(MeasureKey, tf::Size<f32>)>; MEASUREMENTS],
    /// The slot the next new measurement replaces once all are taken.
    next: u8,
}

impl LayoutCache {
    pub(super) const fn new() -> Self {
        Self {
            final_layout: None,
            measurements: [None; MEASUREMENTS],
            next: 0,
        }
    }

    pub(super) fn get(&self, input: &tf::LayoutInput) -> Option<tf::LayoutOutput> {
        match input.run_mode {
            tf::RunMode::PerformLayout => {
                let key = FinalKey::of(input);
                self.final_layout
                    .filter(|(cached, _)| *cached == key)
                    .map(|(_, output)| output)
            }
            tf::RunMode::ComputeSize => {
                let key = MeasureKey::of(input);
                self.measurements
                    .iter()
                    .flatten()
                    .find(|(cached, _)| *cached == key)
                    .map(|(_, size)| tf::LayoutOutput::from_outer_size(*size))
            }
            tf::RunMode::PerformHiddenLayout => None,
        }
    }

    pub(super) fn store(&mut self, input: &tf::LayoutInput, output: tf::LayoutOutput) {
        match input.run_mode {
            tf::RunMode::PerformLayout => {
                self.final_layout = Some((FinalKey::of(input), output));
            }
            tf::RunMode::ComputeSize => {
                let key = MeasureKey::of(input);
                let slot = self
                    .measurements
                    .iter()
                    .position(|entry| entry.is_none_or(|(cached, _)| cached == key))
                    .unwrap_or_else(|| {
                        let slot = usize::from(self.next);
                        self.next = ((slot + 1) % MEASUREMENTS) as u8;
                        slot
                    });
                self.measurements[slot] = Some((key, output.size));
            }
            tf::RunMode::PerformHiddenLayout => {}
        }
    }

    pub(super) fn clear(&mut self) {
        *self = Self::new();
    }

    /// Whether nothing is cached: cleared since the node was last laid
    /// out, or never laid out.
    pub(super) fn is_empty(&self) -> bool {
        self.final_layout.is_none() && self.measurements.iter().all(Option::is_none)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(
        run_mode: tf::RunMode,
        available: tf::Size<tf::AvailableSpace>,
        parent_width: Option<f32>,
    ) -> tf::LayoutInput {
        tf::LayoutInput {
            run_mode,
            sizing_mode: tf::SizingMode::ContentSize,
            axis: tf::RequestedAxis::Both,
            known_dimensions: tf::Size::NONE,
            parent_size: tf::Size {
                width: parent_width,
                height: None,
            },
            available_space: available,
            vertical_margins_are_collapsible: tf::Line::FALSE,
        }
    }

    fn output(width: f32) -> tf::LayoutOutput {
        tf::LayoutOutput::from_outer_size(tf::Size { width, height: 1.0 })
    }

    const MAX: tf::Size<tf::AvailableSpace> = tf::Size {
        width: tf::AvailableSpace::Definite(900.0),
        height: tf::AvailableSpace::MaxContent,
    };

    #[test]
    fn measurements_of_one_kind_with_different_parents_both_stay() {
        let mut cache = LayoutCache::new();
        let unsized_parent = input(tf::RunMode::ComputeSize, MAX, None);
        let sized_parent = input(tf::RunMode::ComputeSize, MAX, Some(900.0));
        cache.store(&unsized_parent, output(1.0));
        cache.store(&sized_parent, output(2.0));

        assert_eq!(cache.get(&unsized_parent).map(|o| o.size.width), Some(1.0));
        assert_eq!(cache.get(&sized_parent).map(|o| o.size.width), Some(2.0));
    }

    #[test]
    fn a_measurement_does_not_answer_a_final_layout() {
        let mut cache = LayoutCache::new();
        cache.store(&input(tf::RunMode::ComputeSize, MAX, None), output(1.0));
        assert!(
            cache
                .get(&input(tf::RunMode::PerformLayout, MAX, None))
                .is_none()
        );
    }

    #[test]
    fn the_oldest_measurement_makes_room() {
        let mut cache = LayoutCache::new();
        let at = |width: f32| {
            input(
                tf::RunMode::ComputeSize,
                tf::Size {
                    width: tf::AvailableSpace::Definite(width),
                    height: tf::AvailableSpace::MaxContent,
                },
                None,
            )
        };
        for width in 0..=MEASUREMENTS {
            cache.store(&at(width as f32), output(width as f32));
        }
        assert!(cache.get(&at(0.0)).is_none(), "the first was replaced");
        for width in 1..=MEASUREMENTS {
            assert!(cache.get(&at(width as f32)).is_some());
        }
    }

    #[test]
    fn clearing_forgets_everything() {
        let mut cache = LayoutCache::new();
        let measure = input(tf::RunMode::ComputeSize, MAX, None);
        let layout = input(tf::RunMode::PerformLayout, MAX, None);
        cache.store(&measure, output(1.0));
        cache.store(&layout, output(1.0));
        cache.clear();
        assert!(cache.get(&measure).is_none());
        assert!(cache.get(&layout).is_none());
    }
}
