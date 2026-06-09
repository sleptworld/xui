use std::{fmt::Debug, hash::Hash};

use slotmap::new_key_type;
use smallstr::SmallString;

use crate::{
    ComputedStyle, ComputedTextStyle, DirtyFlags, Event, EventContext, EventResult, PaintCommand,
    Rect, Size, Style, TextContent, TextLayoutConstraints, WidgetState,
};

new_key_type! {
    pub struct NodeId;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WidgetType {
    Text,
    Label,
    Button,
    Column,
    Row,
    Container,
    StyleScope,
    ScrollScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key(pub SmallString<[u8; 64]>);

impl From<&str> for Key {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}

impl From<String> for Key {
    fn from(value: String) -> Self {
        Self(value.into())
    }
}

pub trait TextMeasurer {
    // fn set_scale_factor(&mut self, _scale_factor: f32) {}

    fn measure_text(&mut self, text: &str, props: &ComputedTextStyle) -> Size<f32>;

    fn measure_text_with_constraints(
        &mut self,
        text: &str,
        props: &ComputedTextStyle,
        _constraints: TextLayoutConstraints,
    ) -> Size<f32>;

    fn measure_node_text(
        &mut self,
        _node_id: NodeId,
        text: &str,
        props: &ComputedTextStyle,
    ) -> Size<f32> {
        self.measure_text(text, props)
    }

    fn measure_node_text_with_constraints(
        &mut self,
        _node_id: NodeId,
        text: &str,
        props: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Size<f32> {
        self.measure_text_with_constraints(text, props, constraints)
    }

    fn handle_node_lifecycle(&mut self, _event: &NodeLifecycleEvent) {}
}

pub trait TextLayoutBackend: TextMeasurer {
    type Layout: Clone;
    type GlyphKey: Clone + Eq + Hash;

    fn layout_text(
        &mut self,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Self::Layout;

    fn layout_node_text(
        &mut self,
        _node_id: NodeId,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Self::Layout {
        self.layout_text(text, style, constraints)
    }

    fn get_cached_layout(&self, _node_id: NodeId) -> Option<Self::Layout> {
        None
    }

    fn layout_size(&self, layout: &Self::Layout) -> Size<f32>;

    fn visit_layout_glyphs(
        &self,
        layout: &Self::Layout,
        origin: crate::Point,
        scale_factor: f32,
        visitor: &mut dyn FnMut(PositionedGlyph<Self::GlyphKey>),
    );

    fn rasterize_glyph(&mut self, key: &Self::GlyphKey) -> Option<GlyphBitmap>;
}

#[derive(Clone, Debug)]
pub struct PositionedGlyph<K> {
    pub key: K,
    pub physical_x: i32,
    pub physical_y: i32,
}

#[derive(Clone, Debug)]
pub struct GlyphBitmap {
    pub is_rgba: bool,
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub placement: GlyphPlacement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlyphPlacement {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
}

pub trait Widget: Debug {
    fn node_type(&self) -> WidgetType;

    fn key(&self) -> Option<&Key> {
        None
    }

    fn props_hash(&self) -> u64;

    fn update_from(&mut self, next: &Self) -> DirtyFlags;

    fn default_style(&self) -> Style {
        Style::new()
    }

    fn style(&self) -> &Style {
        static STYLE: std::sync::LazyLock<Style> = std::sync::LazyLock::new(Style::new);
        &STYLE
    }

    fn state_style(&self, _state: WidgetState) -> Style {
        Style::new()
    }

    fn state(&self) -> WidgetState {
        WidgetState::default()
    }

    fn style_scope(&self) -> Option<&Style> {
        None
    }

    fn paint(&self, rect: Rect, style: &ComputedStyle, commands: &mut Vec<PaintCommand>);

    fn handle_event(&mut self, event: &Event, cx: &mut EventContext<'_>) -> EventResult;

    fn on_hovered_change(&mut self, _hovered: bool) -> DirtyFlags {
        DirtyFlags::empty()
    }

    fn on_click(&mut self) {}

    fn text(&self) -> Option<TextContent> {
        None
    }
}

pub trait Component<Context, Output> {
    fn render(&mut self, cx: &mut Context) -> Output;
}

impl<F, Context, Output> Component<Context, Output> for F
where
    F: FnMut(&mut Context) -> Output,
{
    fn render(&mut self, cx: &mut Context) -> Output {
        self(cx)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeLifecycleEvent {
    Created(NodeId),
    Moved {
        id: NodeId,
        old_parent: Option<NodeId>,
        new_parent: Option<NodeId>,
        old_position: usize,
        new_position: usize,
    },
    Removed(NodeId),
}
