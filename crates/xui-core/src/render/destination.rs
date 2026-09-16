use super::{
    BackdropIsolation, BuiltFrame, BuiltItem, BuiltLayerId, BuiltLayerInstanceId, CompositePrefix,
    CompositePrefixId, SurfacePrefix,
};
use rustc_hash::FxHashMap;

#[derive(Debug, Clone, Copy)]
struct PrefixSegment {
    local: SurfacePrefix,
    placement: Option<BuiltLayerInstanceId>,
}

pub(crate) fn build_destination_history(frame: &mut BuiltFrame) {
    frame.composite_prefixes.clear();
    for instance in &mut frame.layer_instances {
        instance.destination_prefix = None;
    }

    let root = frame.root_layer;
    PrefixBuilder {
        frame,
        interned: FxHashMap::default(),
    }
    .visit_layer(root, None, &mut Vec::new());
}

struct PrefixBuilder<'a> {
    frame: &'a mut BuiltFrame,
    interned: FxHashMap<CompositePrefix, CompositePrefixId>,
}

impl PrefixBuilder<'_> {
    fn visit_layer(
        &mut self,
        layer: BuiltLayerId,
        placement: Option<BuiltLayerInstanceId>,
        ancestry: &mut Vec<PrefixSegment>,
    ) {
        let item_count = self.frame.layers[layer.0].items.len();
        for item_index in 0..item_count {
            let instance_id = match &self.frame.layers[layer.0].items[item_index] {
                BuiltItem::Layer(id) => *id,
                BuiltItem::Draw(_) => continue,
            };
            let current = PrefixSegment {
                local: SurfacePrefix {
                    layer,
                    item_count: item_index,
                },
                placement,
            };
            let requires_backdrop = self.frame.layer_instances[instance_id.0]
                .render_program
                .program()
                .external_resource(xui_render_graph::ExternalResourceKind::Backdrop)
                .is_some();
            if requires_backdrop {
                let prefix = self.materialize(ancestry.iter().copied().chain([current]));
                self.frame.layer_instances[instance_id.0].destination_prefix = Some(prefix);
            }

            let child = self.frame.layer_instances[instance_id.0].layer;
            if self.frame.layers[child.0].backdrop_isolation == BackdropIsolation::Isolate {
                self.visit_layer(child, None, &mut Vec::new());
            } else {
                ancestry.push(current);
                self.visit_layer(child, Some(instance_id), ancestry);
                ancestry.pop();
            }
        }
    }

    fn materialize(
        &mut self,
        segments: impl IntoIterator<Item = PrefixSegment>,
    ) -> CompositePrefixId {
        let mut parent = None;
        for segment in segments {
            let node = CompositePrefix {
                parent,
                local: segment.local,
                placement: segment.placement,
            };
            parent = Some(match self.interned.get(&node).copied() {
                Some(id) => id,
                None => {
                    let id = CompositePrefixId(self.frame.composite_prefixes.len());
                    self.frame.composite_prefixes.push(node);
                    self.interned.insert(node, id);
                    id
                }
            });
        }
        parent.expect("a destination prefix always contains the current surface")
    }
}
