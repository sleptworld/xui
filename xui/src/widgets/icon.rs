use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::Arc;

use crate::element::ElementDesc;
use crate::event_system::EventContext;
use crate::event_system::callbacks::EventHandlers;
use xui_interface::{
    Affine, Color, ComputedStyle, EventRef, EventResult, FillRule, Key, PathData, PathFill,
    PathStroke, Rect, Size, Sizing, Style, TextContent, TextProps, VectorSceneBuilder, WidgetType,
    WidgetUpdateFlags,
};

use super::{props_hash, widget_element_desc};
use crate::render::{Primitive, RenderTreeWriter, VectorPrimitive};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IconStroke {
    pub width: f32,
    pub cap: xui_interface::LineCap,
    pub join: xui_interface::LineJoin,
}

impl IconStroke {
    pub const fn new(width: f32) -> Self {
        Self {
            width,
            cap: xui_interface::LineCap::Butt,
            join: xui_interface::LineJoin::Miter,
        }
    }

    pub const fn cap(mut self, cap: xui_interface::LineCap) -> Self {
        self.cap = cap;
        self
    }

    pub const fn join(mut self, join: xui_interface::LineJoin) -> Self {
        self.join = join;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IconLayer {
    pub path: PathData,
    pub fill: Option<FillRule>,
    pub stroke: Option<IconStroke>,
    pub transform: Affine,
}

impl IconLayer {
    pub fn fill(path: PathData) -> Self {
        Self {
            path,
            fill: Some(FillRule::NonZero),
            stroke: None,
            transform: Affine::IDENTITY,
        }
    }

    pub fn stroke(path: PathData, stroke: IconStroke) -> Self {
        Self {
            path,
            fill: None,
            stroke: Some(stroke),
            transform: Affine::IDENTITY,
        }
    }

    pub fn fill_and_stroke(path: PathData, rule: FillRule, stroke: IconStroke) -> Self {
        Self {
            path,
            fill: Some(rule),
            stroke: Some(stroke),
            transform: Affine::IDENTITY,
        }
    }

    pub fn transform(mut self, transform: Affine) -> Self {
        self.transform = transform;
        self
    }
}

#[derive(Debug)]
pub enum SvgIconError {
    Parse(String),
    Io(std::io::Error),
    NoPaths,
}

impl std::fmt::Display for SvgIconError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(error) => write!(f, "failed to parse SVG: {error}"),
            Self::Io(error) => write!(f, "failed to read SVG: {error}"),
            Self::NoPaths => f.write_str("SVG contains no visible paths"),
        }
    }
}

impl std::error::Error for SvgIconError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IconData {
    view_box: Rect,
    layers: Arc<[IconLayer]>,
}

impl IconData {
    pub fn new(view_box: Rect, layers: impl Into<Arc<[IconLayer]>>) -> Self {
        Self {
            view_box,
            layers: layers.into(),
        }
    }

    pub fn from_fill(view_box: Rect, path: PathData) -> Self {
        Self::new(view_box, Arc::from([IconLayer::fill(path)]))
    }

    pub fn from_stroke(view_box: Rect, path: PathData, stroke: IconStroke) -> Self {
        Self::new(view_box, Arc::from([IconLayer::stroke(path, stroke)]))
    }

    pub fn from_svg(svg: &str) -> Result<Self, SvgIconError> {
        Self::from_svg_bytes(svg.as_bytes())
    }

    pub fn from_svg_bytes(svg: &[u8]) -> Result<Self, SvgIconError> {
        let tree = usvg::Tree::from_data(svg, &usvg::Options::default())
            .map_err(|error| SvgIconError::Parse(error.to_string()))?;
        let mut layers = Vec::new();
        collect_svg_layers(tree.root(), &mut layers);
        if layers.is_empty() {
            return Err(SvgIconError::NoPaths);
        }
        let size = tree.size();
        Ok(Self::new(
            Rect::new(0.0, 0.0, size.width(), size.height()),
            Arc::from(layers),
        ))
    }

    pub fn from_svg_file(path: impl AsRef<Path>) -> Result<Self, SvgIconError> {
        let data = std::fs::read(path).map_err(SvgIconError::Io)?;
        Self::from_svg_bytes(&data)
    }

    pub fn view_box(&self) -> Rect {
        self.view_box
    }
    pub fn layers(&self) -> &[IconLayer] {
        &self.layers
    }
}

impl Hash for IconData {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.view_box.x.to_bits().hash(state);
        self.view_box.y.to_bits().hash(state);
        self.view_box.width.to_bits().hash(state);
        self.view_box.height.to_bits().hash(state);
        for layer in self.layers.iter() {
            layer.path.id().hash(state);
            layer.fill.hash(state);
            layer
                .stroke
                .map(|s| (s.width.to_bits(), s.cap, s.join))
                .hash(state);
            for value in [
                layer.transform.xx,
                layer.transform.yx,
                layer.transform.xy,
                layer.transform.yy,
                layer.transform.dx,
                layer.transform.dy,
            ] {
                value.to_bits().hash(state);
            }
        }
    }
}

pub struct IconWidget {
    pub key: Option<Key>,
    pub data: IconData,
    pub color: Option<Color>,
    pub style: Style,
    pub event_handlers: EventHandlers,
}

impl std::fmt::Debug for IconWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IconWidget")
            .field("key", &self.key)
            .field("data", &self.data)
            .field("color", &self.color)
            .field("style", &self.style)
            .finish()
    }
}

impl IconWidget {
    pub fn new(data: IconData) -> Self {
        Self {
            key: None,
            data,
            color: None,
            style: Style::new(),
            event_handlers: EventHandlers::default(),
        }
    }

    pub fn from_svg(svg: &str) -> Result<Self, SvgIconError> {
        IconData::from_svg(svg).map(Self::new)
    }

    pub fn from_svg_bytes(svg: &[u8]) -> Result<Self, SvgIconError> {
        IconData::from_svg_bytes(svg).map(Self::new)
    }

    pub fn from_svg_file(path: impl AsRef<Path>) -> Result<Self, SvgIconError> {
        IconData::from_svg_file(path).map(Self::new)
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }
    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
    pub fn into_element_desc(self) -> ElementDesc {
        widget_element_desc(self, Vec::new())
    }

    event_handler_methods!();
}

impl IconWidget {
    pub(super) fn node_type(&self) -> WidgetType {
        WidgetType::Icon
    }
    pub(super) fn get_key(&self) -> Option<&Key> {
        self.key.as_ref()
    }
    pub(super) fn props_hash(&self) -> u64 {
        props_hash(&(
            &self.data,
            self.color
                .map(|c| (c.r.to_bits(), c.g.to_bits(), c.b.to_bits(), c.a.to_bits())),
            &self.style,
        ))
    }

    pub(super) fn update_from(&mut self, next: &Self) -> WidgetUpdateFlags {
        let mut flags = WidgetUpdateFlags::empty();
        if self.data != next.data || self.color != next.color {
            flags |= WidgetUpdateFlags::PAINT_OUTPUT;
        }
        if self.style != next.style {
            flags |= WidgetUpdateFlags::STYLE_TARGET;
        }
        self.data = next.data.clone();
        self.color = next.color;
        self.style = next.style.clone();
        flags
    }

    pub(super) fn default_style(&self) -> Style {
        Style::new().size(Size::<Sizing>::fix(24.0, 24.0))
    }
    pub(super) fn current_style(&self) -> &Style {
        &self.style
    }

    pub(super) fn render(
        &self,
        _node_id: xui_interface::NodeId,
        rect: Rect,
        style: &ComputedStyle,
        writer: &mut RenderTreeWriter<'_>,
    ) {
        let view = self.data.view_box;
        if rect.width <= 0.0 || rect.height <= 0.0 || view.width <= 0.0 || view.height <= 0.0 {
            return;
        }
        let scale = (rect.width / view.width).min(rect.height / view.height);
        let x = rect.x + (rect.width - view.width * scale) * 0.5;
        let y = rect.y + (rect.height - view.height * scale) * 0.5;
        let transform = Affine::translate(-view.x, -view.y)
            .then(Affine::scale(scale, scale))
            .then(Affine::translate(x, y));
        let color = self.color.unwrap_or(style.text.color);

        let mut scene = VectorSceneBuilder::new();
        for layer in self.data.layers.iter() {
            if let Some(rule) = layer.fill {
                scene.fill_path(
                    layer.path.clone(),
                    layer.transform,
                    PathFill::new(color).rule(rule),
                );
            }
            if let Some(stroke) = layer.stroke {
                scene.stroke_path(
                    layer.path.clone(),
                    layer.transform,
                    PathStroke::new(color, stroke.width)
                        .cap(stroke.cap)
                        .join(stroke.join),
                );
            }
        }
        writer
            .primitive(Primitive::Vector(VectorPrimitive {
                scene: scene.build(),
                transform,
            }))
            .expect("widget render tree must remain valid");
    }

    pub(super) fn handle_event(
        &mut self,
        _event: EventRef<'_>,
        _cx: &mut EventContext<'_>,
    ) -> EventResult {
        EventResult::Ignored
    }
    pub(super) fn text_content(&self) -> Option<TextContent> {
        None
    }
    pub(super) fn text_layout_props(&self, _style: &ComputedStyle) -> Option<TextProps> {
        None
    }
}

fn collect_svg_layers(group: &usvg::Group, layers: &mut Vec<IconLayer>) {
    for node in group.children() {
        match node {
            usvg::Node::Group(group) => collect_svg_layers(group, layers),
            usvg::Node::Path(path) if path.is_visible() => {
                let data = convert_svg_path(path.data());
                if data.is_empty() {
                    continue;
                }
                let fill = path.fill().map(|fill| match fill.rule() {
                    usvg::FillRule::NonZero => FillRule::NonZero,
                    usvg::FillRule::EvenOdd => FillRule::EvenOdd,
                });
                let stroke = path.stroke().map(|stroke| {
                    IconStroke::new(stroke.width().get())
                        .cap(match stroke.linecap() {
                            usvg::LineCap::Butt => xui_interface::LineCap::Butt,
                            usvg::LineCap::Round => xui_interface::LineCap::Round,
                            usvg::LineCap::Square => xui_interface::LineCap::Square,
                        })
                        .join(match stroke.linejoin() {
                            usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => {
                                xui_interface::LineJoin::Miter
                            }
                            usvg::LineJoin::Round => xui_interface::LineJoin::Round,
                            usvg::LineJoin::Bevel => xui_interface::LineJoin::Bevel,
                        })
                });
                let transform = svg_transform(path.abs_transform());
                let layer = IconLayer {
                    path: data,
                    fill,
                    stroke,
                    transform,
                };
                match path.paint_order() {
                    usvg::PaintOrder::FillAndStroke => layers.push(layer),
                    usvg::PaintOrder::StrokeAndFill => {
                        if let Some(stroke) = layer.stroke {
                            layers.push(IconLayer {
                                path: layer.path.clone(),
                                fill: None,
                                stroke: Some(stroke),
                                transform,
                            });
                        }
                        if let Some(fill) = layer.fill {
                            layers.push(IconLayer {
                                path: layer.path,
                                fill: Some(fill),
                                stroke: None,
                                transform,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn convert_svg_path(path: &usvg::tiny_skia_path::Path) -> PathData {
    let mut builder = xui_interface::PathBuilder::new();
    for segment in path.segments() {
        match segment {
            usvg::tiny_skia_path::PathSegment::MoveTo(point) => {
                builder.move_to(xui_interface::Point::new(point.x, point.y));
            }
            usvg::tiny_skia_path::PathSegment::LineTo(point) => {
                builder.line_to(xui_interface::Point::new(point.x, point.y));
            }
            usvg::tiny_skia_path::PathSegment::QuadTo(control, to) => {
                builder.quadratic_to(
                    xui_interface::Point::new(control.x, control.y),
                    xui_interface::Point::new(to.x, to.y),
                );
            }
            usvg::tiny_skia_path::PathSegment::CubicTo(control1, control2, to) => {
                builder.cubic_to(
                    xui_interface::Point::new(control1.x, control1.y),
                    xui_interface::Point::new(control2.x, control2.y),
                    xui_interface::Point::new(to.x, to.y),
                );
            }
            usvg::tiny_skia_path::PathSegment::Close => {
                builder.close();
            }
        }
    }
    builder.build()
}

fn svg_transform(transform: usvg::Transform) -> Affine {
    Affine::new(
        transform.sx,
        transform.ky,
        transform.kx,
        transform.sy,
        transform.tx,
        transform.ty,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use xui_interface::{PathBuilder, Point, StyleValue, Theme};

    fn icon_data() -> IconData {
        let mut path = PathBuilder::new();
        path.move_to(Point::new(0.0, 0.0))
            .line_to(Point::new(10.0, 0.0))
            .line_to(Point::new(10.0, 20.0))
            .close();
        IconData::from_fill(Rect::new(0.0, 0.0, 10.0, 20.0), path.build())
    }

    #[test]
    fn defaults_to_twenty_four_logical_pixels() {
        let style = IconWidget::new(icon_data()).default_style();
        assert_eq!(
            style.base.layout.width,
            StyleValue::Value(Sizing::fix(24.0))
        );
        assert_eq!(
            style.base.layout.height,
            StyleValue::Value(Sizing::fix(24.0))
        );
    }

    #[test]
    fn contain_mapping_centers_view_box_and_uses_explicit_color() {
        let color = Color::rgb(0.2, 0.4, 0.6);
        let widget = IconWidget::new(icon_data()).color(color);
        let style = ComputedStyle::initial(&Theme::default());
        let mut scene = crate::render::RenderScene::new();
        let parent = scene.insert_group();
        let mut writer = RenderTreeWriter::new(&mut scene, parent);
        widget.render(
            xui_interface::NodeId::default(),
            Rect::new(0.0, 0.0, 40.0, 40.0),
            &style,
            &mut writer,
        );
        writer.finish().unwrap();
        let node = scene.node(scene.children(parent).unwrap()[0]).unwrap();
        let crate::render::RenderNodeKind::Primitive(node) = &node.kind else {
            panic!("expected primitive")
        };
        let Primitive::Vector(vector) = &node.primitive else {
            panic!("expected vector")
        };
        let [xui_interface::VectorCommand::FillPath { fill, .. }] = vector.scene.commands() else {
            panic!("expected one fill command")
        };
        assert_eq!(fill.color, color);
        assert_eq!(
            vector.transform.transform_point(Point::new(0.0, 0.0)),
            Point::new(10.0, 0.0)
        );
        assert_eq!(
            vector.transform.transform_point(Point::new(10.0, 20.0)),
            Point::new(30.0, 40.0)
        );
    }

    #[test]
    fn parses_svg_paths_styles_and_transforms() {
        let svg = r#"
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                <g transform="translate(2 3)">
                    <path d="M1 1 L10 1 Q12 4 10 7 Z"
                          fill="none" stroke="currentColor" stroke-width="2"
                          stroke-linecap="round" stroke-linejoin="bevel"/>
                </g>
                <path d="M2 2 H22 V22 H2 Z M7 7 V17 H17 V7 Z"
                      fill="currentColor" fill-rule="evenodd"/>
            </svg>
        "#;
        let icon = IconData::from_svg(svg).unwrap();
        assert_eq!(icon.view_box(), Rect::new(0.0, 0.0, 24.0, 24.0));
        assert_eq!(icon.layers().len(), 2);
        let stroke = icon.layers()[0].stroke.unwrap();
        assert_eq!(stroke.width, 2.0);
        assert_eq!(stroke.cap, xui_interface::LineCap::Round);
        assert_eq!(stroke.join, xui_interface::LineJoin::Bevel);
        assert_eq!(icon.layers()[0].fill, None);
        assert_eq!(
            icon.layers()[0]
                .transform
                .transform_point(Point::new(1.0, 1.0)),
            Point::new(3.0, 4.0)
        );
        assert_eq!(icon.layers()[1].fill, Some(FillRule::EvenOdd));
        assert!(icon.layers()[1].path.segments().len() >= 6);
    }

    #[test]
    fn rejects_invalid_or_pathless_svg() {
        assert!(matches!(
            IconData::from_svg("<svg>"),
            Err(SvgIconError::Parse(_))
        ));
        assert!(matches!(
            IconData::from_svg(r#"<svg xmlns="http://www.w3.org/2000/svg"/>"#),
            Err(SvgIconError::NoPaths)
        ));
    }
}
