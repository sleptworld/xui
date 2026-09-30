//! Positioning a node against another node's on-screen bounds.
//!
//! A Portal given [`PortalDesc::anchor`](crate::element::PortalDesc::anchor)
//! has its content's root taken out of flow and translated next to the host
//! the Portal is written under, every frame. It is how a Portal-mounted menu
//! or popover stays attached to its trigger when the window resizes, the
//! trigger moves, or it scrolls.
//!
//! This module holds the placement vocabulary and the placement math; the
//! runtime side -- which nodes are anchored, their offsets, and the order they
//! are placed in -- is `ui_runtime::anchor::AnchorSystem`.

use std::hash::{Hash, Hasher};

use xui_interface::{Bounds, Point, Size};

/// Which side of the anchor the node is placed on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AnchorSide {
    Top,
    #[default]
    Bottom,
    Left,
    Right,
}

/// How the node lines up with the anchor along that side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AnchorAlign {
    #[default]
    Start,
    Center,
    End,
}

/// Where an anchored node goes relative to its anchor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnchorPlacement {
    pub side: AnchorSide,
    pub align: AnchorAlign,
    /// Gap between the anchor and the node, in logical pixels.
    pub offset: f32,
    /// Give the node the anchor's width, as a select menu matches its trigger.
    pub match_width: bool,
    /// Flip to the opposite side when the preferred one would overflow the
    /// window and the other fits, then keep the node inside the window.
    pub avoid_collisions: bool,
}

impl Default for AnchorPlacement {
    fn default() -> Self {
        Self {
            side: AnchorSide::Bottom,
            align: AnchorAlign::Start,
            offset: 4.0,
            match_width: false,
            avoid_collisions: true,
        }
    }
}

impl Hash for AnchorPlacement {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.side.hash(state);
        self.align.hash(state);
        self.offset.to_bits().hash(state);
        self.match_width.hash(state);
        self.avoid_collisions.hash(state);
    }
}

impl AnchorPlacement {
    pub fn new(side: AnchorSide) -> Self {
        Self {
            side,
            ..Self::default()
        }
    }

    pub fn align(mut self, align: AnchorAlign) -> Self {
        self.align = align;
        self
    }

    pub fn offset(mut self, offset: f32) -> Self {
        self.offset = offset;
        self
    }

    pub fn match_width(mut self, match_width: bool) -> Self {
        self.match_width = match_width;
        self
    }

    pub fn avoid_collisions(mut self, avoid_collisions: bool) -> Self {
        self.avoid_collisions = avoid_collisions;
        self
    }
}

/// The window-space origin for a node of `size` placed against `anchor`.
pub(crate) fn place(
    anchor: Bounds,
    size: Size<f32>,
    viewport: Size<f32>,
    placement: AnchorPlacement,
) -> Point {
    let AnchorPlacement {
        mut side,
        align,
        offset,
        avoid_collisions,
        ..
    } = placement;

    if avoid_collisions {
        let fits = |side: AnchorSide| match side {
            AnchorSide::Bottom => anchor.max.y + offset + size.height <= viewport.height,
            AnchorSide::Top => anchor.min.y - offset - size.height >= 0.0,
            AnchorSide::Right => anchor.max.x + offset + size.width <= viewport.width,
            AnchorSide::Left => anchor.min.x - offset - size.width >= 0.0,
        };
        let opposite = match side {
            AnchorSide::Bottom => AnchorSide::Top,
            AnchorSide::Top => AnchorSide::Bottom,
            AnchorSide::Right => AnchorSide::Left,
            AnchorSide::Left => AnchorSide::Right,
        };
        if !fits(side) && fits(opposite) {
            side = opposite;
        }
    }

    let aligned = |start: f32, anchor_len: f32, len: f32| match align {
        AnchorAlign::Start => start,
        AnchorAlign::Center => start + (anchor_len - len) * 0.5,
        AnchorAlign::End => start + anchor_len - len,
    };
    let mut origin = match side {
        AnchorSide::Bottom => Point::new(
            aligned(anchor.min.x, anchor.width(), size.width),
            anchor.max.y + offset,
        ),
        AnchorSide::Top => Point::new(
            aligned(anchor.min.x, anchor.width(), size.width),
            anchor.min.y - offset - size.height,
        ),
        AnchorSide::Right => Point::new(
            anchor.max.x + offset,
            aligned(anchor.min.y, anchor.height(), size.height),
        ),
        AnchorSide::Left => Point::new(
            anchor.min.x - offset - size.width,
            aligned(anchor.min.y, anchor.height(), size.height),
        ),
    };

    if avoid_collisions {
        let clamp = |value: f32, len: f32, limit: f32| value.min(limit - len).max(0.0);
        origin.x = clamp(origin.x, size.width, viewport.width);
        origin.y = clamp(origin.y, size.height, viewport.height);
    }
    origin
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds {
        Bounds::from_origin_size(Point::new(x, y), (width, height))
    }

    const VIEWPORT: Size<f32> = Size {
        width: 400.0,
        height: 300.0,
    };

    #[test]
    fn places_below_the_anchor_start_aligned() {
        let origin = place(
            bounds(20.0, 40.0, 100.0, 30.0),
            Size::new(160.0, 80.0),
            VIEWPORT,
            AnchorPlacement::default(),
        );
        assert_eq!(origin, Point::new(20.0, 74.0));
    }

    #[test]
    fn flips_above_when_below_overflows_and_above_fits() {
        let origin = place(
            bounds(20.0, 240.0, 100.0, 30.0),
            Size::new(100.0, 120.0),
            VIEWPORT,
            AnchorPlacement::default(),
        );
        assert_eq!(origin, Point::new(20.0, 116.0));
    }

    #[test]
    fn stays_inside_the_window_horizontally() {
        let origin = place(
            bounds(350.0, 40.0, 40.0, 30.0),
            Size::new(160.0, 80.0),
            VIEWPORT,
            AnchorPlacement::default(),
        );
        assert_eq!(origin.x, 240.0);
    }

    #[test]
    fn centers_and_ends_along_the_side() {
        let anchor = bounds(100.0, 40.0, 100.0, 30.0);
        let size = Size::new(40.0, 20.0);
        let center = AnchorPlacement::default().align(AnchorAlign::Center);
        let end = AnchorPlacement::default().align(AnchorAlign::End);
        assert_eq!(place(anchor, size, VIEWPORT, center).x, 130.0);
        assert_eq!(place(anchor, size, VIEWPORT, end).x, 160.0);
    }
}
