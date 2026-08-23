use std::{cell::RefCell, rc::Rc};

use crate::element::ElementDesc;
use crate::event_system::interaction::InteractionProperties;
use crate::event_system::{EventContext, callbacks::EventHandlers};
use crate::render::{ClipShape, Primitive, RenderTreeWriter, TextPrimitive, VectorPrimitive};
use crate::text::TextLayoutSlot;
use xui_interface::{
    Affine, Bounds, CanvasTextId, ComputedStyle, EventRef, EventResult, Key, Rect, Style,
    TextContent, TextPaintProps, TextPaintStyle, TextProps, VectorCommand, VectorScene, WidgetType,
    WidgetUpdateFlags,
};

use super::{props_hash, widget_element_desc};

#[derive(Debug, Clone)]
struct CanvasControllerState {
    scene: VectorScene,
    revision: u64,
}

/// Shared retained vector content for a [`CanvasWidget`].
///
/// Like `TextController`, this controller only owns data. Mutating it does not
/// wake the runtime by itself; the next component rebuild synchronizes the
/// controller revision into every canvas using the handle.
#[derive(Clone)]
pub struct CanvasController {
    inner: Rc<RefCell<CanvasControllerState>>,
}

impl std::fmt::Debug for CanvasController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanvasController")
            .field("scene", &self.scene())
            .field("revision", &self.revision())
            .finish_non_exhaustive()
    }
}

impl CanvasController {
    pub fn new() -> Self {
        Self::with_scene(VectorScene::default())
    }

    pub fn with_scene(scene: VectorScene) -> Self {
        Self {
            inner: Rc::new(RefCell::new(CanvasControllerState { scene, revision: 0 })),
        }
    }

    pub fn scene(&self) -> VectorScene {
        self.inner.borrow().scene.clone()
    }

    pub fn revision(&self) -> u64 {
        self.inner.borrow().revision
    }

    pub fn same_handle(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    pub fn set_scene(&self, scene: VectorScene) {
        self.update(|current| *current = scene);
    }

    pub fn update(&self, update: impl FnOnce(&mut VectorScene)) {
        let mut state = self.inner.borrow_mut();
        let previous = state.scene.clone();
        update(&mut state.scene);
        if state.scene != previous {
            state.revision = state.revision.wrapping_add(1);
        }
    }

    pub fn clear(&self) {
        self.set_scene(VectorScene::default());
    }

    fn identity(&self) -> usize {
        Rc::as_ptr(&self.inner) as usize
    }
}

impl Default for CanvasController {
    fn default() -> Self {
        Self::new()
    }
}

pub struct CanvasWidget {
    pub key: Option<Key>,
    pub controller: CanvasController,
    pub style: Style,
    pub event_handlers: EventHandlers,
    pub interaction: InteractionProperties,
    last_revision: u64,
}

impl std::fmt::Debug for CanvasWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanvasWidget")
            .field("key", &self.key)
            .field("controller", &self.controller)
            .field("style", &self.style)
            .field("last_revision", &self.last_revision)
            .finish()
    }
}

impl CanvasWidget {
    pub fn new(controller: CanvasController) -> Self {
        let last_revision = controller.revision();
        Self {
            key: None,
            controller,
            style: Style::new(),
            event_handlers: EventHandlers::default(),
            interaction: InteractionProperties::default(),
            last_revision,
        }
    }

    pub fn controller(mut self, controller: CanvasController) -> Self {
        self.last_revision = controller.revision();
        self.controller = controller;
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

impl CanvasWidget {
    pub(super) fn node_type(&self) -> WidgetType {
        WidgetType::Canvas
    }

    pub(super) fn get_key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    pub(super) fn props_hash(&self) -> u64 {
        props_hash(&(self.controller.identity(), self.last_revision, &self.style))
    }

    pub(super) fn update_from(&mut self, next: &Self) -> WidgetUpdateFlags {
        let mut flags = WidgetUpdateFlags::empty();
        let next_revision = next.controller.revision();

        if !self.controller.same_handle(&next.controller) {
            self.controller = next.controller.clone();
            flags |= WidgetUpdateFlags::PAINT_OUTPUT | WidgetUpdateFlags::TEXT_SHAPE;
        }

        if self.last_revision != next_revision {
            flags |= WidgetUpdateFlags::PAINT_OUTPUT | WidgetUpdateFlags::TEXT_SHAPE;
        }
        self.last_revision = next_revision;

        if self.style != next.style {
            self.style = next.style.clone();
            flags |= WidgetUpdateFlags::STYLE_TARGET;
        }

        flags
    }

    pub(super) fn default_style(&self) -> Style {
        Style::new()
    }

    pub(super) fn current_style(&self) -> &Style {
        &self.style
    }

    pub(super) fn render(
        &self,
        node_id: xui_interface::NodeId,
        rect: Bounds,
        _style: &ComputedStyle,
        writer: &mut RenderTreeWriter<'_>,
    ) {
        let scene = self.controller.scene();
        writer
            .clip(ClipShape::Rect(rect), |writer| {
                let mut vector_commands = Vec::new();
                for command in scene.commands() {
                    match command {
                        VectorCommand::FillPath { .. } | VectorCommand::StrokePath { .. } => {
                            vector_commands.push(command.clone());
                        }
                        VectorCommand::TextBox { id, bounds, props } => {
                            flush_vector_commands(writer, &mut vector_commands, rect)?;
                            let bounds = bounds.translate(rect.origin());
                            let paint = TextPaintProps::new(TextPaintStyle {
                                color: props.style.color,
                                font_size: props.style.font_size,
                                line_height: props.style.line_height,
                                decoration: props.style.decoration,
                            });
                            writer.clip(ClipShape::Rect(bounds), |writer| {
                                writer.primitive(Primitive::Text(TextPrimitive {
                                    node_id,
                                    bounds,
                                    slot: canvas_text_slot(*id),
                                    layout_revision: self.last_revision,
                                    vertical_align: props.paragraph.vertical_align,
                                    paint,
                                }))?;
                                Ok(())
                            })?;
                        }
                    }
                }
                flush_vector_commands(writer, &mut vector_commands, rect)?;
                Ok(())
            })
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

pub(crate) fn canvas_text_slot(id: CanvasTextId) -> TextLayoutSlot {
    TextLayoutSlot::new(id.get())
}

fn flush_vector_commands(
    writer: &mut RenderTreeWriter<'_>,
    commands: &mut Vec<VectorCommand>,
    rect: Bounds,
) -> Result<(), crate::render::SceneError> {
    if commands.is_empty() {
        return Ok(());
    }
    let scene = VectorScene::new(std::mem::take(commands));
    writer.primitive(Primitive::Vector(VectorPrimitive {
        scene,
        transform: Affine::translate(rect.x(), rect.y()),
    }))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{RenderNodeKind, RenderScene};
    use slotmap::Key as _;
    use xui_interface::{
        CanvasTextId, Color, PathBuilder, PathFill, PathStroke, Point, TextProps,
        VectorSceneBuilder,
    };

    fn scene(color: Color) -> VectorScene {
        let mut path = PathBuilder::new();
        path.move_to(Point::new(0.0, 0.0))
            .line_to(Point::new(10.0, 10.0));
        let mut scene = VectorSceneBuilder::new();
        scene.fill_path(path.build(), Affine::IDENTITY, PathFill::new(color));
        scene.build()
    }

    #[test]
    fn controller_updates_shared_scene_and_revision_only_on_change() {
        let controller = CanvasController::new();
        let clone = controller.clone();
        assert!(controller.same_handle(&clone));
        assert_eq!(controller.revision(), 0);

        let first = scene(Color::BLACK);
        controller.set_scene(first.clone());
        assert_eq!(clone.scene(), first);
        assert_eq!(controller.revision(), 1);

        controller.set_scene(first);
        assert_eq!(controller.revision(), 1);
        controller.update(|current| *current = scene(Color::WHITE));
        assert_eq!(controller.revision(), 2);
        controller.clear();
        assert_eq!(controller.revision(), 3);
        controller.clear();
        assert_eq!(controller.revision(), 3);
    }

    #[test]
    fn widget_diff_tracks_revision_snapshot_and_handle_identity() {
        let controller = CanvasController::new();
        let mut current = CanvasWidget::new(controller.clone());
        let initial_hash = current.props_hash();

        controller.set_scene(scene(Color::BLACK));
        assert_eq!(
            current.props_hash(),
            initial_hash,
            "the current widget hash must remain an observed revision snapshot"
        );
        let next = CanvasWidget::new(controller.clone());
        assert_ne!(initial_hash, next.props_hash());
        assert_eq!(
            current.update_from(&next),
            WidgetUpdateFlags::PAINT_OUTPUT | WidgetUpdateFlags::TEXT_SHAPE
        );
        assert!(current.update_from(&next).is_empty());

        let replacement = CanvasWidget::new(CanvasController::with_scene(controller.scene()));
        assert_eq!(
            current.update_from(&replacement),
            WidgetUpdateFlags::PAINT_OUTPUT | WidgetUpdateFlags::TEXT_SHAPE
        );
    }

    #[test]
    fn shared_controller_revisions_are_observed_by_each_widget() {
        let controller = CanvasController::new();
        let mut first = CanvasWidget::new(controller.clone());
        let mut second = CanvasWidget::new(controller.clone());

        controller.set_scene(scene(Color::BLACK));
        let next = CanvasWidget::new(controller);

        assert_eq!(
            first.update_from(&next),
            WidgetUpdateFlags::PAINT_OUTPUT | WidgetUpdateFlags::TEXT_SHAPE
        );
        assert_eq!(
            second.update_from(&next),
            WidgetUpdateFlags::PAINT_OUTPUT | WidgetUpdateFlags::TEXT_SHAPE
        );
    }

    #[test]
    fn render_uses_host_clip_and_local_origin_transform() {
        let widget = CanvasWidget::new(CanvasController::with_scene(scene(Color::BLACK)));
        let mut render_scene = RenderScene::new();
        let parent = render_scene.insert_group();
        let rect = Bounds::from_origin_size((12.0, 18.0), (80.0, 40.0));
        let style = ComputedStyle::initial(&crate::style::Theme::default());

        let mut writer = RenderTreeWriter::new(&mut render_scene, parent);
        widget.render(xui_interface::NodeId::null(), rect, &style, &mut writer);
        writer.finish().unwrap();

        let clip = render_scene.children(parent).unwrap()[0];
        let RenderNodeKind::Clip(clip_node) = &render_scene.node(clip).unwrap().kind else {
            panic!("canvas root should be a clip");
        };
        assert_eq!(clip_node.clip, ClipShape::Rect(rect));
        let group = clip_node.child.unwrap();
        let primitive = render_scene.children(group).unwrap()[0];
        let RenderNodeKind::Primitive(primitive) = &render_scene.node(primitive).unwrap().kind
        else {
            panic!("canvas clip should contain a vector primitive");
        };
        let Primitive::Vector(vector) = &primitive.primitive else {
            panic!("canvas should render a vector primitive");
        };
        assert_eq!(vector.transform, Affine::translate(rect.x(), rect.y()));
    }

    #[test]
    fn render_preserves_vector_text_vector_order_and_text_slot() {
        let mut path = PathBuilder::new();
        path.move_to(Point::new(0.0, 0.0))
            .line_to(Point::new(10.0, 10.0));
        let path = path.build();
        let mut scene = VectorSceneBuilder::new();
        scene
            .fill_path(path.clone(), Affine::IDENTITY, PathFill::new(Color::BLACK))
            .text_box(
                CanvasTextId::new(42),
                Bounds::from_origin_size((10.0, 12.0), (80.0, 24.0)),
                TextProps::new("canvas"),
            )
            .stroke_path(path, Affine::IDENTITY, PathStroke::new(Color::WHITE, 1.0));
        let widget = CanvasWidget::new(CanvasController::with_scene(scene.build()));
        let mut render_scene = RenderScene::new();
        let parent = render_scene.insert_group();
        let rect = Bounds::from_origin_size((2.0, 3.0), (100.0, 50.0));
        let style = ComputedStyle::initial(&crate::style::Theme::default());

        let mut writer = RenderTreeWriter::new(&mut render_scene, parent);
        widget.render(xui_interface::NodeId::null(), rect, &style, &mut writer);
        writer.finish().unwrap();

        let outer_clip = render_scene.children(parent).unwrap()[0];
        let RenderNodeKind::Clip(outer) = &render_scene.node(outer_clip).unwrap().kind else {
            panic!("canvas root should be a clip");
        };
        let children = render_scene.children(outer.child.unwrap()).unwrap();
        assert_eq!(children.len(), 3);
        assert!(matches!(
            render_scene.node(children[0]).unwrap().kind,
            RenderNodeKind::Primitive(_)
        ));
        let RenderNodeKind::Clip(text_clip) = &render_scene.node(children[1]).unwrap().kind else {
            panic!("text box should introduce its own clip");
        };
        let text_node = render_scene.children(text_clip.child.unwrap()).unwrap()[0];
        let RenderNodeKind::Primitive(text) = &render_scene.node(text_node).unwrap().kind else {
            panic!("text clip should contain a primitive");
        };
        let Primitive::Text(text) = &text.primitive else {
            panic!("middle command should be text");
        };
        assert_eq!(text.slot, TextLayoutSlot::new(42));
        // assert_eq!(text.bounds, Rect::new(12.0, 15.0, 80.0, 24.0));
        assert!(matches!(
            render_scene.node(children[2]).unwrap().kind,
            RenderNodeKind::Primitive(_)
        ));
    }
}
