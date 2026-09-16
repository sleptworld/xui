use slotmap::SecondaryMap;
use xui_interface::{Bounds, NodeId};

use crate::render::{
    FrameBuilder, FrameProperties, HostRenderBinding, RenderScene, SceneCompiler, SceneError,
};

pub(crate) struct RenderSystem {
    pub scene: RenderScene,
    pub compiler: SceneCompiler,
    pub builder: FrameBuilder,
    pub properties: FrameProperties,
    pub last_viewport: Option<Bounds>,
    bindings: SecondaryMap<NodeId, HostRenderBinding>,
}

impl RenderSystem {
    pub fn new() -> Self {
        Self {
            scene: RenderScene::new(),
            compiler: SceneCompiler::new(),
            builder: FrameBuilder::new(),
            properties: FrameProperties::default(),
            last_viewport: None,
            bindings: SecondaryMap::new(),
        }
    }

    pub(crate) fn bind_host(
        &mut self,
        host: NodeId,
        binding: HostRenderBinding,
    ) -> Result<Option<HostRenderBinding>, SceneError> {
        for (field, id) in binding.references() {
            if !self.scene.contains(id) {
                return Err(SceneError::InvalidHostBinding { field, node: id });
            }
        }
        Ok(self.bindings.insert(host, binding))
    }

    pub(crate) fn unbind_host(&mut self, host: NodeId) -> Option<HostRenderBinding> {
        self.bindings.remove(host)
    }

    pub(crate) fn host_binding(&self, host: NodeId) -> Option<&HostRenderBinding> {
        self.bindings.get(host)
    }

    pub(crate) fn host_binding_mut(&mut self, host: NodeId) -> Option<&mut HostRenderBinding> {
        self.bindings.get_mut(host)
    }
}

impl Default for RenderSystem {
    fn default() -> Self {
        Self::new()
    }
}
