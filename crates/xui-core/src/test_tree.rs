//! Static host trees for tests that need hosts and no component: a draft
//! with its children, mounted as written.
use xui_interface::NodeId;

use crate::app::App;
use crate::node::Draft;
use crate::ui_runtime::UiRuntime;
use crate::widgets::ErasedDraft;

pub(crate) struct StaticTree {
    draft: Box<dyn ErasedDraft>,
    children: Vec<StaticTree>,
}

pub(crate) trait IntoTree {
    /// This draft with `children` under it.
    fn into_tree(self, children: Vec<StaticTree>) -> StaticTree;
}

impl<N> IntoTree for Draft<N>
where
    Draft<N>: ErasedDraft + 'static,
    N: crate::node::NodeDef,
{
    fn into_tree(self, children: Vec<StaticTree>) -> StaticTree {
        StaticTree { draft: Box::new(self), children }
    }
}

impl UiRuntime {
    /// Mounts `tree` last under `parent`; returns its root.
    pub(crate) fn mount_static(&mut self, parent: NodeId, tree: StaticTree) -> NodeId {
        let id = self.create_node(tree.draft);
        self.place(parent, id, None);
        for child in tree.children {
            self.mount_static(id, child);
        }
        id
    }
}

impl App {
    /// Mounts `tree` at the root.
    pub(crate) fn mount_static(&mut self, tree: StaticTree) -> NodeId {
        let ui = self.ui_runtime_mut();
        let root = ui.root();
        ui.mount_static(root, tree)
    }
}
