//! State the runtime publishes about the window's surface, for answers a native
//! callback needs on the spot -- `WM_NCHITTEST`, `draggingUpdated:`, an
//! input-method query.
//!
//! Such a callback can arrive while the shell is in the middle of an event, so
//! it must be answered from published state, not by calling back into the
//! runtime. A host reads this through the shell's handle at any moment.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use crate::{Point, Rect};

use super::command::ResizeEdge;

/// What a point on the window means to the platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowRegion {
    /// Ordinary content; the runtime gets the pointer.
    Content,
    /// A title bar: dragging moves the window, double-clicking zooms it.
    Caption,
    Resize(ResizeEdge),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegionRect {
    pub rect: Rect,
    pub region: WindowRegion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DropAccepts {
    pub files: bool,
    pub text: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DropEffect {
    #[default]
    None,
    Copy,
    Move,
    Link,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DropTarget {
    pub rect: Rect,
    pub accepts: DropAccepts,
    pub effect: DropEffect,
}

/// What a drag carries, as far as a drop target can tell from the outside.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DragPayload {
    pub files: Vec<PathBuf>,
    pub text: Option<String>,
}

impl DragPayload {
    pub fn kinds(&self) -> DropAccepts {
        DropAccepts {
            files: !self.files.is_empty(),
            text: self.text.is_some(),
        }
    }
}

/// A drag crossing the window, delivered as ordinary input. The *answer* the
/// platform wants while it is in flight comes from [`PublishedSurface::drop_effect_at`].
#[derive(Debug, Clone, PartialEq)]
pub enum DragEvent {
    Enter {
        payload: DragPayload,
        position: Point,
    },
    Over {
        position: Point,
    },
    Leave,
    Drop {
        payload: DragPayload,
        position: Point,
    },
}

/// The focused text input's document, for the system's text services
/// (reconversion, dictation, look-up). Offsets are UTF-16 code units, which is
/// what every native text-input protocol speaks.
#[derive(Debug, Clone, PartialEq)]
pub struct TextDocument {
    pub text: String,
    pub selection: Range<usize>,
    pub marked: Option<Range<usize>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PublishedSurface {
    /// Later entries win. Anything not covered is [`WindowRegion::Content`].
    pub regions: Arc<[RegionRect]>,
    pub drop_targets: Arc<[DropTarget]>,
    pub text_document: Option<Arc<TextDocument>>,
}

impl Default for PublishedSurface {
    fn default() -> Self {
        Self {
            regions: Arc::from([]),
            drop_targets: Arc::from([]),
            text_document: None,
        }
    }
}

impl PublishedSurface {
    pub fn region_at(&self, point: Point) -> WindowRegion {
        self.regions
            .iter()
            .rev()
            .find(|entry| contains(entry.rect, point))
            .map_or(WindowRegion::Content, |entry| entry.region)
    }

    /// The effect a drag carrying `kinds` would have at `point`: what a host
    /// returns from `draggingUpdated:` or `DragOver`.
    pub fn drop_effect_at(&self, point: Point, kinds: DropAccepts) -> DropEffect {
        self.drop_targets
            .iter()
            .rev()
            .find(|target| contains(target.rect, point))
            .filter(|target| {
                (!kinds.files || target.accepts.files) && (!kinds.text || target.accepts.text)
            })
            .map_or(DropEffect::None, |target| target.effect)
    }
}

fn contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.x
        && point.y >= rect.y
        && point.x < rect.x + rect.width
        && point.y < rect.y + rect.height
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn region_defaults_to_content_and_later_entries_win() {
        let surface = PublishedSurface {
            regions: Arc::from([
                RegionRect {
                    rect: rect(0.0, 0.0, 100.0, 30.0),
                    region: WindowRegion::Caption,
                },
                RegionRect {
                    rect: rect(80.0, 0.0, 20.0, 30.0),
                    region: WindowRegion::Content,
                },
            ]),
            ..PublishedSurface::default()
        };
        assert_eq!(
            surface.region_at(Point::new(10.0, 10.0)),
            WindowRegion::Caption
        );
        // A button inside the title bar punches a hole in it.
        assert_eq!(
            surface.region_at(Point::new(90.0, 10.0)),
            WindowRegion::Content
        );
        assert_eq!(
            surface.region_at(Point::new(10.0, 50.0)),
            WindowRegion::Content
        );
    }

    #[test]
    fn drop_effect_requires_every_carried_kind_to_be_accepted() {
        let surface = PublishedSurface {
            drop_targets: Arc::from([DropTarget {
                rect: rect(0.0, 0.0, 50.0, 50.0),
                accepts: DropAccepts {
                    files: true,
                    text: false,
                },
                effect: DropEffect::Copy,
            }]),
            ..PublishedSurface::default()
        };
        let inside = Point::new(10.0, 10.0);
        let files = DropAccepts {
            files: true,
            text: false,
        };
        let both = DropAccepts {
            files: true,
            text: true,
        };
        assert_eq!(surface.drop_effect_at(inside, files), DropEffect::Copy);
        assert_eq!(surface.drop_effect_at(inside, both), DropEffect::None);
        assert_eq!(
            surface.drop_effect_at(Point::new(60.0, 10.0), files),
            DropEffect::None
        );
    }
}
