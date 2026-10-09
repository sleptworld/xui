//! `PathData` -> `BezPath`, kept between frames.
//!
//! A canvas or an icon hands the same path to the renderer every frame, and
//! converting it is a walk over every segment. Cached by the path's id, and
//! weighed by segment count so one huge path cannot crowd out thousands of
//! small ones.

use std::sync::Arc;

use moka::sync::Cache;
use rustc_hash::FxBuildHasher;
use vello::kurbo::BezPath;
use xui_interface::{PathData, PathDataId, PathSegment};

/// Segments held in total; a segment is a few dozen bytes once converted.
const BUDGET_SEGMENTS: u64 = 1_000_000;

pub(crate) struct PathCache {
    cache: Cache<PathDataId, Arc<BezPath>, FxBuildHasher>,
}

impl PathCache {
    pub(crate) fn new(budget_segments: u64) -> Self {
        Self {
            cache: Cache::builder()
                .max_capacity(budget_segments)
                .weigher(|_: &PathDataId, path: &Arc<BezPath>| {
                    u32::try_from(path.elements().len()).unwrap_or(u32::MAX).max(1)
                })
                .build_with_hasher(FxBuildHasher),
        }
    }

    pub(crate) fn get(&self, path: &PathData) -> Arc<BezPath> {
        self.cache
            .get_with(path.id(), || Arc::new(kurbo_path(path)))
    }

    /// Applies pending evictions, so the budget holds before the next frame.
    pub(crate) fn settle(&self) {
        self.cache.run_pending_tasks();
    }

    pub(crate) fn entries(&self) -> u64 {
        self.cache.entry_count()
    }

    pub(crate) fn segments(&self) -> u64 {
        self.cache.weighted_size()
    }
}

impl Default for PathCache {
    fn default() -> Self {
        Self::new(BUDGET_SEGMENTS)
    }
}

pub(crate) fn kurbo_path(path: &PathData) -> BezPath {
    let mut out = BezPath::new();
    for segment in path.segments() {
        match *segment {
            PathSegment::MoveTo(p) => out.move_to((p.x as f64, p.y as f64)),
            PathSegment::LineTo(p) => out.line_to((p.x as f64, p.y as f64)),
            PathSegment::QuadraticTo { control, to } => {
                out.quad_to((control.x as f64, control.y as f64), (to.x as f64, to.y as f64))
            }
            PathSegment::CubicTo { control1, control2, to } => out.curve_to(
                (control1.x as f64, control1.y as f64),
                (control2.x as f64, control2.y as f64),
                (to.x as f64, to.y as f64),
            ),
            PathSegment::Close => out.close_path(),
        }
    }
    out
}
