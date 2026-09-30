//! The paint pass: widget repaint, canvases, and the retained render scene.

use crate::core::Point;
use crate::render::{
    ClipShape, HostRenderBinding, LayerDescriptor, Primitive, RenderTreeWriter, SceneError, Shape,
    ShapePrimitive,
};
use crate::text::TextHost;
use crate::ui_runtime::scroll::{scrollbar_parts, should_paint_scrollbar};
use crate::ui_runtime::state::HostWorkFlags;
use crate::ui_runtime::{NodeView, UiRuntime};
use crate::widgets::{Widgets, canvas_text_slot};
use xui_interface::{
    Affine, Bounds, ComputedColorStyle, ComputedStyle, NodeId, ScrollbarVisibilityStyle,
    TextBackend, TextLayoutInput, TextProps,
};

impl UiRuntime {
    #[inline(always)]
    pub(super) fn sync_render_scene(&mut self) -> Result<(), SceneError> {
        self.sync_render_dirty_subtree(self.root)
    }

    fn sync_render_dirty_subtree(&mut self, id: NodeId) -> Result<(), SceneError> {
        let relevant = HostWorkFlags::RENDER;
        let work = self.hosts[id].work;
        let subtree_work = self.hosts[id].subtree_work;
        if !work.intersects(relevant) && !subtree_work.intersects(relevant) {
            return Ok(());
        }
        if work.intersects(HostWorkFlags::SCENE) {
            self.sync_host_render_node(id)?;
        }
        // Independent of the scene sync: the transform node a binding writes
        // to is created with the host and never replaced.
        if work.intersects(HostWorkFlags::SYNC_TRANSFORM) {
            self.sync_effective_transform(id);
        }
        // `sync_host_render_node` only rewrites the render scene, so the host
        // sibling chain stays valid while it is walked in place.
        let mut cursor = self.hosts.link(id).first_child;
        while let Some(child) = cursor {
            cursor = self.hosts.link(child).next_sibling;
            let child_work = self.hosts[child].work | self.hosts[child].subtree_work;
            if !child_work.intersects(relevant) {
                continue;
            }
            self.sync_render_dirty_subtree(child)?;
        }
        Ok(())
    }

    fn sync_host_render_node(&mut self, id: NodeId) -> Result<(), SceneError> {
        let mut binding = self.render_system.binding(id);
        let host_children: Vec<_> = self.hosts.children(id).collect();

        let (
            local_origin,
            viewport,
            scroll,
            host_children,
            needs_scroll,
            needs_overlay,
            clip_shape,
            layer_descriptor,
        ) = {
            let host = &self.hosts[id];
            let layout = self.layout_tree.node(id);
            let (target, effective) = self.style_system.styles(id);
            let node = NodeView::new(id, host, layout, target, effective);
            let viewport = Bounds::from_origin_size(Point::zero(), layout.layout.size());
            let needs_scroll = effective.scroll.is_scrollable();
            let needs_clip = node.effective_style.paint.clip || needs_scroll;
            let clip_shape = needs_clip.then_some({
                if node.effective_style.paint.border_radius > 0.0 {
                    ClipShape::RoundedRect {
                        rect: viewport,
                        radius: node.effective_style.paint.border_radius,
                    }
                } else {
                    ClipShape::Rect(viewport)
                }
            });

            (
                layout.layout.origin(),
                viewport,
                layout.scroll_offset,
                host_children,
                needs_scroll,
                needs_scrollbar_overlay(node),
                clip_shape,
                layer_descriptor_from_style(node.effective_style, viewport),
            )
        };

        // Reconcile the fixed host scaffold.
        self.render_system.scene.update_transform(
            binding.root,
            Affine::translate(local_origin.x, local_origin.y),
        )?;

        self.update_wrappers(id, &mut binding, clip_shape, layer_descriptor)?;

        // The children group is a stable container once it has been created.
        if !host_children.is_empty() && binding.children.is_none() {
            let children = self.render_system.scene.insert_group();
            // `paint` is always at index 0. Insert the child branch at index 1
            // so a scrollbar overlay remains last.
            self.render_system
                .scene
                .insert_child(binding.contents, 1, children)?;
            binding.children = Some(children);
        }

        // Scroll transforms are transient and exist only while scrolling is enabled.
        let removed_scroll_transform = if needs_scroll {
            if binding.scroll_transform.is_none()
                && let Some(children) = binding.children
            {
                self.render_system.scene.detach(children)?;
                let scroll_transform = self.render_system.scene.insert_transform(Affine::IDENTITY);
                self.render_system
                    .scene
                    .set_child(scroll_transform, Some(children))?;
                self.render_system
                    .scene
                    .insert_child(binding.contents, 1, scroll_transform)?;
                binding.scroll_transform = Some(scroll_transform);
            }
            None
        } else if let Some(scroll_transform) = binding.scroll_transform.take() {
            if let Some(children) = binding.children {
                self.render_system.scene.set_child(scroll_transform, None)?;
                self.render_system.scene.detach(scroll_transform)?;
                self.render_system
                    .scene
                    .insert_child(binding.contents, 1, children)?;
            }
            self.render_system
                .properties
                .remove_source(scroll_transform);
            Some(scroll_transform)
        } else {
            None
        };

        // Scrollbar overlays are also transient.
        let removed_overlay = if needs_overlay {
            if binding.overlay.is_none() {
                let overlay = self.render_system.scene.insert_group();
                self.render_system
                    .scene
                    .append_child(binding.contents, overlay)?;
                binding.overlay = Some(overlay);
            }
            None
        } else if let Some(overlay) = binding.overlay.take() {
            self.render_system.scene.detach(overlay)?;
            Some(overlay)
        } else {
            None
        };

        // Remove transient nodes only after the host binding stops referencing them.
        *self.render_system.binding_mut(id) = binding;

        if let Some(scroll_transform) = removed_scroll_transform {
            self.render_system.scene.remove_subtree(scroll_transform)?;
        }
        if let Some(overlay) = removed_overlay {
            self.render_system.scene.remove_subtree(overlay)?;
        }

        // Scroll offsets remain dynamic frame properties while scrolling is active.
        if let Some(scroll_transform) = binding.scroll_transform {
            self.render_system
                .properties
                .set_transform(scroll_transform, Affine::translate(-scroll.x, -scroll.y));
        }

        // Reconcile host children in declaration/paint order.
        if let Some(children_binding) = binding.children {
            let children_match = self
                .render_system
                .scene
                .children(children_binding)?
                .iter()
                .copied()
                .eq(host_children
                    .iter()
                    .map(|host_child| self.render_system.binding(*host_child).root));

            if !children_match {
                let current = self
                    .render_system
                    .scene
                    .children(children_binding)?
                    .to_vec();

                for child_root in current {
                    self.render_system.scene.detach(child_root)?;
                }

                for host_child in &host_children {
                    let child_root = self.render_system.binding(*host_child).root;

                    self.render_system.scene.detach(child_root)?;
                    self.render_system
                        .scene
                        .append_child(children_binding, child_root)?;
                }
            }
        }

        if let Some(overlay) = binding.overlay {
            let (target, effective) = self.style_system.styles(id);
            let node = NodeView::new(
                id,
                &self.hosts[id],
                self.layout_tree.node(id),
                target,
                effective,
            );
            let mut writer = RenderTreeWriter::new(&mut self.render_system.scene, overlay);
            render_scrollbars_in_rect(node, viewport, &mut writer);
            writer.finish()?;
        }

        Ok(())
    }

    /// Re-derives `id`'s paint transform. Only the render sync calls this,
    /// for nodes marked `SYNC_TRANSFORM`, so it runs at most once a frame
    /// and always sees the settled style, anchor offset, and size.
    fn sync_effective_transform(&mut self, id: NodeId) {
        self.stats.transform_syncs += 1;
        let binding = self.render_system.binding(id);
        let style = self.style_system.effective(id).transform;
        let offset = self.anchor_offset(id);
        if style == xui_interface::TransformStyle::IDENTITY && offset == Point::zero() {
            self.render_system
                .properties
                .clear_transform(binding.transform);
            return;
        }
        let size = self.layout_tree.node(id).layout.size();
        // An anchor translates after the style transform, as if the container
        // had been laid out where the anchor put it.
        let transform = style
            .to_affine(size)
            .then(Affine::translate(offset.x, offset.y));
        self.render_system
            .properties
            .set_transform(binding.transform, transform);
    }

    fn update_wrappers(
        &mut self,
        id: NodeId,
        binding: &mut HostRenderBinding,
        clip_shape: Option<ClipShape>,
        layer_descriptor: Option<LayerDescriptor>,
    ) -> Result<(), SceneError> {
        let topology_changed = clip_shape.is_some() != binding.clip.is_some()
            || layer_descriptor.is_some() != binding.layer.is_some();

        let old_clip = binding.clip;
        let old_layer = binding.layer;

        // Reuse existing wrappers and update their descriptors in place.
        let removed_clip = match clip_shape {
            Some(shape) => {
                if let Some(clip) = binding.clip {
                    self.render_system.scene.update_clip(clip, shape)?;
                } else {
                    binding.clip = Some(self.render_system.scene.insert_clip(shape));
                }
                None
            }
            None => binding.clip.take(),
        };

        let removed_layer = match layer_descriptor {
            Some(descriptor) => {
                if let Some(layer) = binding.layer {
                    self.render_system
                        .scene
                        .update_layer_descriptor(layer, descriptor)?;
                } else {
                    binding.layer = Some(self.render_system.scene.insert_layer(descriptor));
                }
                None
            }
            None => binding.layer.take(),
        };

        if !topology_changed {
            return Ok(());
        }

        // Break the old transform -> clip -> layer -> contents chain. The
        // layout root remains permanently attached to the style transform.
        self.render_system
            .scene
            .set_child(binding.transform, None)?;
        if let Some(clip) = old_clip {
            self.render_system.scene.set_child(clip, None)?;
        }
        if let Some(layer) = old_layer {
            self.render_system.scene.set_child(layer, None)?;
        }

        // Rebuild the wrapper chain from inside out.
        let mut child = binding.contents;
        if let Some(layer) = binding.layer {
            self.render_system.scene.set_child(layer, Some(child))?;
            child = layer;
        }
        if let Some(clip) = binding.clip {
            self.render_system.scene.set_child(clip, Some(child))?;
            child = clip;
        }
        self.render_system
            .scene
            .set_child(binding.transform, Some(child))?;

        // Drop binding references before removing obsolete wrapper subtrees.
        let stored = self.render_system.binding_mut(id);

        stored.clip = binding.clip;
        stored.layer = binding.layer;

        if let Some(clip) = removed_clip {
            self.render_system.scene.remove_subtree(clip)?;
        }

        if let Some(layer) = removed_layer {
            self.render_system.scene.remove_subtree(layer)?;
        }

        Ok(())
    }

    fn repaint(&mut self, id: NodeId) {
        self.stats.repaint_passes += 1;
        let layout = self.layout_tree.node(id).layout;
        let rect = Bounds::from_zero_size(layout.size());
        let style = self.style_system.effective(id).clone();
        let widget = self.hosts[id].widget.clone();
        let paint = self.render_system.binding(id).paint;
        let mut writer = RenderTreeWriter::new(&mut self.render_system.scene, paint);
        widget.render(id, rect, &style, &mut writer);
        writer
            .finish()
            .expect("widget emitted an invalid retained render tree");
    }

    pub(super) fn repaint_dirty_subtree(&mut self, id: NodeId) {
        let work = self.hosts[id].work;
        let subtree_work = self.hosts[id].subtree_work;
        if !work.intersects(HostWorkFlags::REBUILD_PAINT)
            && !subtree_work.intersects(HostWorkFlags::REBUILD_PAINT)
        {
            return;
        }

        if work.intersects(HostWorkFlags::REBUILD_PAINT) {
            self.repaint(id);
        }

        let mut cursor = self.hosts.link(id).first_child;
        while let Some(child) = cursor {
            cursor = self.hosts.link(child).next_sibling;
            let child_work = self.hosts[child].work | self.hosts[child].subtree_work;
            if !child_work.intersects(HostWorkFlags::REBUILD_PAINT) {
                continue;
            }
            self.repaint_dirty_subtree(child);
        }
    }

    /// Rebuilds every canvas whose drawing no longer matches its node.
    ///
    /// Runs after layout and before paint, the same window
    /// `activate_final_text_layouts` uses: a painter needs the size Taffy just
    /// committed, and the text boxes it produces have to reach the shaper
    /// before the node is repainted.
    pub(super) fn sync_dirty_canvases<T: TextBackend>(&mut self, measurer: &mut TextHost<T>) {
        let dirty = self.ui_state.drain_canvas_dirty_list();
        if dirty.is_empty() {
            return;
        }
        let mut compiled = Vec::with_capacity(dirty.len());
        for id in dirty {
            if !self.canvas_nodes.contains_key(id) || compiled.contains(&id) {
                continue;
            }
            compiled.push(id);
            self.compile_canvas(id, measurer);
        }
    }

    fn compile_canvas<T: TextBackend>(&mut self, id: NodeId, measurer: &mut TextHost<T>) {
        let size = self.layout_tree.node(id).layout.size();
        let style = self.style_system.effective(id).clone();
        let theme = self.theme.clone();
        let scale_factor = self.scale_factor;
        let frame_time = self.frame_time;
        let gpu_context = self.gpu_context.clone();
        let widget = self.hosts[id].widget.clone();
        let font_context = measurer.backend().epoch();

        let mut wants_repaint = false;
        let text_boxes = widget.with_widgets_mut(|node| match node {
            Widgets::Canvas(canvas) => {
                let mut measure_text = |text_id, props: &TextProps, constraints| {
                    let input = TextLayoutInput::new(
                        props.text.clone(),
                        constraints,
                        (&props.style).into(),
                        props.paragraph.clone(),
                        props.text_box.clone(),
                        font_context,
                    );
                    let metrics =
                        measurer.measure_slot_metrics(id, canvas_text_slot(text_id), input);
                    crate::widgets::CanvasTextMetrics {
                        size: metrics.size,
                        first_baseline: metrics.first_baseline,
                        line_count: metrics.line_count,
                    }
                };
                canvas.compile(
                    crate::widgets::CanvasFrame {
                        size,
                        style: &style,
                        theme: &theme,
                        scale_factor,
                        time: frame_time,
                        gpu: gpu_context.as_ref(),
                    },
                    &mut measure_text,
                );
                wants_repaint = canvas.wants_repaint();
                canvas.text_boxes()
            }
            _ => Vec::new(),
        });
        if wants_repaint {
            self.canvases_wanting_repaint.insert(id, ());
        } else {
            self.canvases_wanting_repaint.remove(id);
        }

        measurer.retain_direct_slots(
            id,
            text_boxes
                .iter()
                .map(|(text_id, _, _, _)| canvas_text_slot(*text_id)),
        );
        for (text_id, _bounds, props, constraints) in text_boxes {
            let input = TextLayoutInput::new(
                props.text,
                constraints,
                props.style.into(),
                props.paragraph,
                props.text_box,
                font_context,
            );
            measurer.get_or_shape_slot(id, canvas_text_slot(text_id), input);
        }

        self.mark_work(id, HostWorkFlags::REBUILD_PAINT);
    }
}

fn layer_descriptor_from_style(style: &ComputedStyle, bounds: Bounds) -> Option<LayerDescriptor> {
    let descriptor = LayerDescriptor {
        bounds: Some(bounds),
        backdrop_style: style.effect.backdrop.clone(),
        effects: style.effect.effects.clone(),
        ..Default::default()
    };

    descriptor.requires_isolation().then_some(descriptor)
}

fn needs_scrollbar_overlay(node: NodeView<'_>) -> bool {
    let direction = node.effective_style.scroll.direction;
    let scrollbar = node.effective_style.scroll.scrollbar;
    if scrollbar.visibility == ScrollbarVisibilityStyle::Hidden
        || scrollbar.width <= 0.0
        || !scrollbar.thumb_color.is_visible()
    {
        return false;
    }

    let max_x = (node.content_size.width - node.layout.width()).max(0.0);
    let max_y = (node.content_size.height - node.layout.height()).max(0.0);
    (direction.allows_vertical() && should_paint_scrollbar(scrollbar, max_y))
        || (direction.allows_horizontal() && should_paint_scrollbar(scrollbar, max_x))
}

fn render_scrollbars_in_rect(node: NodeView<'_>, rect: Bounds, writer: &mut RenderTreeWriter<'_>) {
    let scroll = node.effective_style.scroll;
    let scrollbar = scroll.scrollbar;
    let parts = scrollbar_parts(rect, scroll, node.content_size, node.scroll_offset);
    for part in parts.into_iter().flatten() {
        render_scrollbar_part(part.track, scrollbar.track_color, scrollbar.radius, writer);
        if let Some(thumb) = part.thumb {
            render_scrollbar_part(thumb, scrollbar.thumb_color, scrollbar.radius, writer);
        }
    }
}

fn render_scrollbar_part(
    rect: Bounds,
    color: ComputedColorStyle,
    radius: f32,
    writer: &mut RenderTreeWriter<'_>,
) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 || !color.is_visible() {
        return;
    }

    let shape = if radius > 0.0 {
        Shape::RoundedRect(radius)
    } else {
        Shape::Rect
    };
    writer
        .primitive(Primitive::Shape(ShapePrimitive {
            bounds: rect,
            shape,
            fill: Some(color),
            stroke: None,
            shadow: None,
        }))
        .expect("scrollbar render tree must remain valid");
}
