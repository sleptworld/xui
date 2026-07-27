use crate::wgpu::{SCENE_FORMAT, SCENE_SAMPLE_COUNT};

pub(super) struct SceneTexture {
    _texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    _msaa_texture: wgpu::Texture,
    pub msaa_view: wgpu::TextureView,
}

impl SceneTexture {
    pub fn new(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> Self {
        let width = config.width.max(1);
        let height = config.height.max(1);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("xui scene cache"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SCENE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let msaa_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("xui multisampled scene cache"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: SCENE_SAMPLE_COUNT,
            dimension: wgpu::TextureDimension::D2,
            format: SCENE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let msaa_view = msaa_texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _texture: texture,
            view,
            _msaa_texture: msaa_texture,
            msaa_view,
        }
    }
}
