//! Widget roles: behaviour a widget's own code implements, as opposed to the
//! data it holds (its capabilities).
//!
//! Each role is a trait a widget implements by hand, and only if it has that
//! behaviour: a widget with nothing of its own to paint does not implement
//! [`PaintContent`] rather than implementing it as a no-op. Nothing here is
//! generated; the logic cannot be — it is what makes a text a text.
//!
//! What is generated is the dispatch. Without specialization nothing can ask
//! a type whether it implements a trait, so a widget declares its roles in
//! one word each (`define_widgets!`'s `[paint, text]`), and [`WidgetRoles`]
//! hands the runtime each declared role as a trait object and `None` for the
//! rest. A declared role the widget does not implement fails to compile.

use taffy::prelude as tf;
use xui_interface::{Bounds, EventRef, EventResult, NodeId, Size, TextContent, TextProps};

use crate::event_system::EventContext;
use crate::layout::ParentLayout;
use crate::render::RenderTreeWriter;
use crate::style::ComputedStyle;
use crate::text::TextLayoutQuery;

/// Paints what is the widget's own — glyphs, an image, a canvas — over the
/// decoration the runtime has already painted.
pub(crate) trait PaintContent {
    fn paint(
        &self,
        node_id: NodeId,
        rect: Bounds,
        style: &ComputedStyle,
        writer: &mut RenderTreeWriter<'_>,
    );
}

/// Text the runtime shapes and lays out for the widget.
pub(crate) trait TextSource {
    fn text(&self) -> TextContent;

    /// The layout input for [`text`](Self::text) under `style`.
    fn text_layout(&self, style: &ComputedStyle) -> TextProps;
}

/// The widget's own reaction to events: not its user handlers (those are
/// the `events` capability), but what the widget does itself.
pub(crate) trait HandleEvent {
    fn handle_event(&mut self, event: EventRef<'_>, cx: &mut EventContext<'_>) -> EventResult;

    /// Whether it reads `EventRef::Raw`. Raw events are widget-private, so
    /// the widgets answering `true` are every node raw dispatch has to visit.
    fn raw_events(&self) -> bool {
        false
    }
}

/// A text input session the platform edits through its IME.
pub(crate) trait PlatformTextInput {
    fn text_input_session(
        &self,
        content_rect: Bounds,
        text_layout: &dyn TextLayoutQuery,
    ) -> xui_interface::TextInputSession;
}

/// A size the widget has of itself, before layout: an image's pixel size.
/// Layout also keeps its aspect ratio.
pub(crate) trait IntrinsicSize {
    fn intrinsic_size(&self) -> Option<Size<f32>>;
}

/// How the widget lays out its children. A widget without it lays them out
/// as a block.
pub(crate) trait ArrangeChildren {
    fn arrangement(&self) -> ParentLayout;

    /// The taffy style that arrangement takes, before the computed style's
    /// box properties (margin, padding, size, ..) go on top.
    fn container_style(&self, computed: &ComputedStyle) -> tf::Style;
}

/// Whether the widget takes focus when its `focus` capability leaves it to
/// the widget (`Focusability::Auto`): a text field does, unless disabled.
pub(crate) trait DefaultFocus {
    fn focusable(&self) -> bool;
}

/// A widget's declared roles, as trait objects. Implemented by
/// `define_widgets!` from each widget's role list; every role it does not
/// declare answers `None`.
pub(crate) trait WidgetRoles {
    fn as_paint(&self) -> Option<&dyn PaintContent> {
        None
    }

    fn as_text(&self) -> Option<&dyn TextSource> {
        None
    }

    fn as_handle_event(&self) -> Option<&dyn HandleEvent> {
        None
    }

    fn as_handle_event_mut(&mut self) -> Option<&mut dyn HandleEvent> {
        None
    }

    fn as_text_input(&self) -> Option<&dyn PlatformTextInput> {
        None
    }

    fn as_intrinsic_size(&self) -> Option<&dyn IntrinsicSize> {
        None
    }

    fn as_arrange(&self) -> Option<&dyn ArrangeChildren> {
        None
    }

    fn as_default_focus(&self) -> Option<&dyn DefaultFocus> {
        None
    }
}
