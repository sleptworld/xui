pub(super) fn choose_srgb_surface_format(
    default: wgpu::TextureFormat,
    supported: &[wgpu::TextureFormat],
) -> Option<wgpu::TextureFormat> {
    let default_srgb = default.add_srgb_suffix();
    if supported.contains(&default_srgb) {
        return Some(default_srgb);
    }

    if default.is_srgb() {
        return Some(default);
    }

    supported.iter().copied().find(wgpu::TextureFormat::is_srgb)
}
