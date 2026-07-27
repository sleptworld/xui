use super::{ClipShape, Primitive, RenderNodeId, RenderNodeKind, RenderScene, SceneError};
use xui_interface::Affine;

/// Reconciles a widget's declarative render output directly into a retained
/// scene group. Compatible nodes are updated in place and keep their stable
/// `RenderNodeId`; incompatible or stale nodes are replaced/removed.
pub(crate) struct RenderTreeWriter<'a> {
    scene: &'a mut RenderScene,
    parent: RenderNodeId,
    cursor: usize,
}

impl<'a> RenderTreeWriter<'a> {
    pub(crate) fn new(scene: &'a mut RenderScene, parent: RenderNodeId) -> Self {
        Self {
            scene,
            parent,
            cursor: 0,
        }
    }

    pub(crate) fn primitive(&mut self, primitive: Primitive) -> Result<RenderNodeId, SceneError> {
        let node = self.ensure_node(NodeKind::Primitive, |scene| {
            scene.insert_primitive(primitive.clone())
        })?;
        self.scene.update_primitive(node, primitive)?;
        Ok(node)
    }

    pub(crate) fn transform(
        &mut self,
        transform: Affine,
        build: impl FnOnce(&mut RenderTreeWriter<'_>) -> Result<(), SceneError>,
    ) -> Result<RenderNodeId, SceneError> {
        let node = self.ensure_node(NodeKind::Transform, |scene| {
            let node = scene.insert_transform(transform);
            let group = scene.insert_group();
            scene
                .set_child(node, Some(group))
                .expect("new transform accepts its child group");
            node
        })?;
        self.scene.update_transform(node, transform)?;
        self.write_container(node, build)?;
        Ok(node)
    }

    pub(crate) fn clip(
        &mut self,
        clip: ClipShape,
        build: impl FnOnce(&mut RenderTreeWriter<'_>) -> Result<(), SceneError>,
    ) -> Result<RenderNodeId, SceneError> {
        let node = self.ensure_node(NodeKind::Clip, |scene| {
            let node = scene.insert_clip(clip.clone());
            let group = scene.insert_group();
            scene
                .set_child(node, Some(group))
                .expect("new clip accepts its child group");
            node
        })?;
        self.scene.update_clip(node, clip)?;
        self.write_container(node, build)?;
        Ok(node)
    }

    pub(crate) fn finish(mut self) -> Result<(), SceneError> {
        self.remove_stale_children()
    }

    fn write_container(
        &mut self,
        node: RenderNodeId,
        build: impl FnOnce(&mut RenderTreeWriter<'_>) -> Result<(), SceneError>,
    ) -> Result<(), SceneError> {
        let group =
            self.scene
                .children(node)?
                .first()
                .copied()
                .ok_or(SceneError::ChildNotFound {
                    parent: node,
                    child: node,
                })?;
        let mut child_writer = RenderTreeWriter::new(self.scene, group);
        build(&mut child_writer)?;
        child_writer.finish()
    }

    fn ensure_node(
        &mut self,
        kind: NodeKind,
        create: impl FnOnce(&mut RenderScene) -> RenderNodeId,
    ) -> Result<RenderNodeId, SceneError> {
        let current = self.scene.children(self.parent)?.get(self.cursor).copied();
        let node = if let Some(current) = current {
            if kind.matches(&self.scene.node(current).expect("scene child exists").kind) {
                current
            } else {
                let replacement = create(self.scene);
                self.scene
                    .replace_child(self.parent, current, replacement)?;
                self.scene.remove_subtree(current)?;
                replacement
            }
        } else {
            let child = create(self.scene);
            self.scene.append_child(self.parent, child)?;
            child
        };
        self.cursor += 1;
        Ok(node)
    }

    fn remove_stale_children(&mut self) -> Result<(), SceneError> {
        let stale = self.scene.children(self.parent)?[self.cursor..].to_vec();
        for node in stale.into_iter().rev() {
            self.scene.remove_subtree(node)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum NodeKind {
    Primitive,
    Transform,
    Clip,
}

impl NodeKind {
    fn matches(self, kind: &RenderNodeKind) -> bool {
        matches!(
            (self, kind),
            (Self::Primitive, RenderNodeKind::Primitive(_))
                | (Self::Transform, RenderNodeKind::Transform(_))
                | (Self::Clip, RenderNodeKind::Clip(_))
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{Shape, ShapePrimitive};
    use xui_interface::{Color, ComputedColorStyle, Rect};

    fn shape(bounds: Rect, color: Color) -> Primitive {
        Primitive::Shape(ShapePrimitive {
            bounds,
            shape: Shape::Rect,
            fill: Some(ComputedColorStyle::Solid(color)),
            stroke: None,
            shadow: None,
        })
    }

    #[test]
    fn compatible_nodes_are_reused_and_stale_nodes_are_removed() {
        let mut scene = RenderScene::new();
        let parent = scene.insert_group();
        {
            let mut writer = RenderTreeWriter::new(&mut scene, parent);
            writer
                .primitive(shape(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLACK))
                .unwrap();
            writer
                .primitive(shape(Rect::new(10.0, 0.0, 10.0, 10.0), Color::WHITE))
                .unwrap();
            writer.finish().unwrap();
        }
        let first = scene.children(parent).unwrap()[0];
        let stale = scene.children(parent).unwrap()[1];
        {
            let mut writer = RenderTreeWriter::new(&mut scene, parent);
            writer
                .primitive(shape(Rect::new(1.0, 0.0, 10.0, 10.0), Color::WHITE))
                .unwrap();
            writer.finish().unwrap();
        }
        assert_eq!(scene.children(parent).unwrap(), &[first]);
        assert!(!scene.contains(stale));
    }
}
