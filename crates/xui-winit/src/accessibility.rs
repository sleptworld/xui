use accesskit::{
    Action, HasPopup, Invalid, Live, Node, NodeId, Orientation, Rect, Role, Toggled, TreeId,
    TreeInfo, TreeUpdate,
};
use std::collections::HashMap;
use xui_interface::{
    AccessibilityChecked, AccessibilityLiveRegion, AccessibilityOrientation, AccessibilityPopup,
    AccessibilityProperties, AccessibilityRole, AccessibilityTree,
};

pub(crate) fn node_id(id: xui_interface::NodeId) -> NodeId {
    NodeId(id.as_u64())
}

pub(crate) fn xui_node_id(id: NodeId) -> xui_interface::NodeId {
    xui_interface::NodeId::from_u64(id.0)
}

pub(crate) fn tree_update(tree: &AccessibilityTree) -> Option<TreeUpdate> {
    let root = tree.root?;
    let ids_by_accessibility_id: HashMap<_, _> = tree
        .nodes
        .iter()
        .filter_map(|node| {
            node.properties
                .id
                .as_ref()
                .map(|id| (id.as_str(), node_id(node.id)))
        })
        .collect();
    let focus = tree
        .nodes
        .iter()
        .find(|node| node.focused)
        .map(|node| node_id(node.id))
        .unwrap_or_else(|| node_id(root));
    let nodes = tree
        .nodes
        .iter()
        .map(|snapshot| {
            let mut node = Node::new(role(snapshot.properties.role));
            node.set_children(
                snapshot
                    .children
                    .iter()
                    .copied()
                    .map(node_id)
                    .collect::<Vec<_>>(),
            );
            node.set_bounds(Rect {
                x0: snapshot.bounds.x() as f64,
                y0: snapshot.bounds.y() as f64,
                x1: (snapshot.bounds.x() + snapshot.bounds.width()) as f64,
                y1: (snapshot.bounds.y() + snapshot.bounds.height()) as f64,
            });
            apply_properties(&mut node, &snapshot.properties, &ids_by_accessibility_id);
            if !snapshot.properties.disabled.unwrap_or(false)
                && matches!(
                    snapshot.properties.role,
                    Some(
                        AccessibilityRole::Button
                            | AccessibilityRole::ComboBox
                            | AccessibilityRole::MenuItem
                            | AccessibilityRole::Option
                            | AccessibilityRole::Tab
                    )
                )
            {
                node.add_action(Action::Click);
            }
            if !snapshot.properties.disabled.unwrap_or(false) && snapshot.focusable {
                node.add_action(Action::Focus);
            }
            (node_id(snapshot.id), node)
        })
        .collect();
    let mut info = TreeInfo::new(node_id(root));
    info.toolkit_name = Some("xui".to_owned());
    info.toolkit_version = Some(env!("CARGO_PKG_VERSION").to_owned());
    Some(TreeUpdate {
        nodes,
        tree: Some(info),
        tree_id: TreeId::ROOT,
        focus,
    })
}

fn role(role: Option<AccessibilityRole>) -> Role {
    match role.unwrap_or(AccessibilityRole::Generic) {
        AccessibilityRole::Generic | AccessibilityRole::Group => Role::GenericContainer,
        AccessibilityRole::Button => Role::Button,
        AccessibilityRole::ComboBox => Role::ComboBox,
        AccessibilityRole::Checkbox => Role::CheckBox,
        AccessibilityRole::Dialog => Role::Dialog,
        AccessibilityRole::Heading => Role::Heading,
        AccessibilityRole::Image => Role::Image,
        AccessibilityRole::Label => Role::Label,
        AccessibilityRole::Link => Role::Link,
        AccessibilityRole::List => Role::List,
        AccessibilityRole::ListBox => Role::ListBox,
        AccessibilityRole::ListItem => Role::ListItem,
        AccessibilityRole::Menu => Role::Menu,
        AccessibilityRole::MenuItem => Role::MenuItem,
        AccessibilityRole::Option => Role::ListBoxOption,
        AccessibilityRole::ProgressIndicator => Role::ProgressIndicator,
        AccessibilityRole::Radio => Role::RadioButton,
        AccessibilityRole::RadioGroup => Role::RadioGroup,
        AccessibilityRole::Slider => Role::Slider,
        AccessibilityRole::Switch => Role::Switch,
        AccessibilityRole::Tab => Role::Tab,
        AccessibilityRole::TabList => Role::TabList,
        AccessibilityRole::TabPanel => Role::TabPanel,
        AccessibilityRole::Table => Role::Table,
        AccessibilityRole::Row => Role::Row,
        AccessibilityRole::Cell => Role::Cell,
        AccessibilityRole::ColumnHeader => Role::ColumnHeader,
        AccessibilityRole::Text => Role::Label,
        AccessibilityRole::TextField => Role::TextInput,
        AccessibilityRole::Tooltip => Role::Tooltip,
        AccessibilityRole::Tree => Role::Tree,
        AccessibilityRole::TreeItem => Role::TreeItem,
    }
}

fn apply_properties(
    node: &mut Node,
    properties: &AccessibilityProperties,
    ids_by_accessibility_id: &HashMap<&str, NodeId>,
) {
    if let Some(label) = &properties.label {
        node.set_label(label.clone());
    }
    if let Some(description) = &properties.description {
        node.set_description(description.clone());
    }
    if let Some(value) = &properties.value {
        node.set_value(value.clone());
    }
    if properties.disabled == Some(true) {
        node.set_disabled();
    }
    if properties.read_only == Some(true) {
        node.set_read_only();
    }
    if properties.required == Some(true) {
        node.set_required();
    }
    if properties.invalid == Some(true) {
        node.set_invalid(Invalid::True);
    }
    if let Some(selected) = properties.selected {
        node.set_selected(selected);
    }
    if let Some(expanded) = properties.expanded {
        node.set_expanded(expanded);
    }
    if let Some(checked) = properties.checked {
        node.set_toggled(match checked {
            AccessibilityChecked::False => Toggled::False,
            AccessibilityChecked::True => Toggled::True,
            AccessibilityChecked::Mixed => Toggled::Mixed,
        });
    }
    if let Some(value) = properties.numeric_value {
        node.set_numeric_value(value.into_inner());
    }
    if let Some(value) = properties.min_value {
        node.set_min_numeric_value(value.into_inner());
    }
    if let Some(value) = properties.max_value {
        node.set_max_numeric_value(value.into_inner());
    }
    if let Some(value) = properties.step_value {
        node.set_numeric_value_step(value.into_inner());
    }
    if let Some(orientation) = properties.orientation {
        node.set_orientation(match orientation {
            AccessibilityOrientation::Horizontal => Orientation::Horizontal,
            AccessibilityOrientation::Vertical => Orientation::Vertical,
        });
    }
    if let Some(popup) = properties.has_popup {
        node.set_has_popup(match popup {
            AccessibilityPopup::Menu => HasPopup::Menu,
            AccessibilityPopup::ListBox => HasPopup::Listbox,
            AccessibilityPopup::Tree => HasPopup::Tree,
            AccessibilityPopup::Grid => HasPopup::Grid,
            AccessibilityPopup::Dialog => HasPopup::Dialog,
        });
    }
    if let Some(live) = properties.live_region {
        node.set_live(match live {
            AccessibilityLiveRegion::Polite => Live::Polite,
            AccessibilityLiveRegion::Assertive => Live::Assertive,
        });
    }
    if let Some(id) = properties
        .controls
        .as_deref()
        .and_then(|id| ids_by_accessibility_id.get(id))
    {
        node.set_controls(vec![*id]);
    }
    if let Some(id) = properties
        .labelled_by
        .as_deref()
        .and_then(|id| ids_by_accessibility_id.get(id))
    {
        node.set_labelled_by(vec![*id]);
    }
    if let Some(id) = properties
        .described_by
        .as_deref()
        .and_then(|id| ids_by_accessibility_id.get(id))
    {
        node.set_described_by(vec![*id]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xui_interface::{AccessibilityNode, Bounds, NodeId as XuiNodeId, Point, Size};

    #[test]
    fn converts_roles_states_and_focus() {
        let id = XuiNodeId::from_u64(7);
        let tree = AccessibilityTree {
            root: Some(id),
            nodes: vec![AccessibilityNode {
                id,
                parent: None,
                children: Vec::new(),
                bounds: Bounds::from_origin_size(Point::new(1.0, 2.0), Size::new(30.0, 40.0)),
                focused: true,
                focusable: true,
                properties: AccessibilityProperties::default()
                    .role(AccessibilityRole::Button)
                    .label("Save")
                    .read_only(true)
                    .required(true)
                    .invalid(true)
                    .disabled(false),
            }],
        };
        let update = tree_update(&tree).unwrap();
        assert_eq!(update.focus, node_id(id));
        assert_eq!(xui_node_id(update.focus), id);
        assert_eq!(update.nodes[0].1.role(), Role::Button);
        assert_eq!(update.nodes[0].1.label(), Some("Save"));
        assert!(update.nodes[0].1.is_read_only());
        assert!(update.nodes[0].1.is_required());
        assert_eq!(update.nodes[0].1.invalid(), Some(Invalid::True));
        assert!(update.nodes[0].1.supports_action(Action::Click));
    }
}
