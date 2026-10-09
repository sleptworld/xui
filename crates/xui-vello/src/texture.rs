//! GPU textures an application rendered itself, drawn from a canvas.
//!
//! Two ways in, as with `xui-skia`: a canvas GPU painter sends its target along
//! with the command (`source`), or the application registers a texture under
//! an id with [`crate::VelloBackend::set_wgpu_texture`] and the command only
//! names it.
//!
//! Vello is not involved in drawing one. It rasterizes 2D content; a foreign
//! texture is composed by this crate's own pass, between the bands of 2D
//! content on either side of it in paint order, sampled where it is with
//! nothing copied. See `Stage` in `encode.rs` for how a target is cut.

use rustc_hash::FxHashMap;
use xui_interface::{ExternalTextureHandle, ExternalTextureId};

#[derive(Default)]
pub(crate) struct ExternalTextures {
    entries: FxHashMap<ExternalTextureId, wgpu::Texture>,
}

impl ExternalTextures {
    /// Registers (or replaces) the texture drawn for `id`.
    pub(crate) fn set(&mut self, id: ExternalTextureId, texture: &wgpu::Texture) {
        self.entries.insert(id, texture.clone());
    }

    pub(crate) fn remove(&mut self, id: ExternalTextureId) -> bool {
        self.entries.remove(&id).is_some()
    }

    /// The texture to draw for a command. A command that carries its own
    /// texture (a canvas GPU painter's target) replaces what was registered,
    /// so a resize that reallocates it is picked up. `None` when nothing is
    /// registered, which is a normal state at startup.
    pub(crate) fn resolve(
        &mut self,
        id: ExternalTextureId,
        source: &Option<ExternalTextureHandle>,
    ) -> Option<wgpu::Texture> {
        if let Some(texture) = source
            .as_ref()
            .and_then(|handle| handle.downcast_ref::<wgpu::Texture>())
            && self.entries.get(&id) != Some(texture)
        {
            self.entries.insert(id, texture.clone());
        }
        self.entries.get(&id).cloned()
    }
}
