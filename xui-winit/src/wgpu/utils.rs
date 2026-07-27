use xui::Affine;

pub(super) fn affine_inverse(transform: Affine) -> Option<Affine> {
    let determinant = transform.xx * transform.yy - transform.xy * transform.yx;
    if determinant.abs() <= f32::EPSILON {
        return None;
    }
    let inverse = 1.0 / determinant;
    let xx = transform.yy * inverse;
    let xy = -transform.xy * inverse;
    let yx = -transform.yx * inverse;
    let yy = transform.xx * inverse;
    Some(Affine::new(
        xx,
        yx,
        xy,
        yy,
        -(xx * transform.dx + xy * transform.dy),
        -(yx * transform.dx + yy * transform.dy),
    ))
}

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
