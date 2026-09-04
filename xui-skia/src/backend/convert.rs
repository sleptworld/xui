//! Small conversions between `xui-interface` values and their `skia-safe`
//! counterparts, plus the geometry helpers that go with them.

use skia_safe::{ColorSpace, Matrix, Rect as SkRect};
use xui_interface::{Affine, Bounds, Rect};

pub(super) fn physical_extent(bounds: Bounds, scale: f32) -> (u32, u32) {
    (
        (bounds.width().max(0.0) * scale).ceil().max(1.0) as u32,
        (bounds.height().max(0.0) * scale).ceil().max(1.0) as u32,
    )
}


pub(super) fn inverse_affine(value: Affine) -> Option<Affine> {
    let determinant = value.xx * value.yy - value.xy * value.yx;
    if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
        return None;
    }
    let xx = value.yy / determinant;
    let xy = -value.xy / determinant;
    let yx = -value.yx / determinant;
    let yy = value.xx / determinant;
    Some(Affine::new(
        xx,
        xy,
        yx,
        yy,
        -(xx * value.dx + xy * value.dy),
        -(yx * value.dx + yy * value.dy),
    ))
}

pub(super) fn valid_scale(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        1.0
    }
}

pub(super) fn sk_bounds(rect: Bounds) -> SkRect {
    SkRect::from_xywh(rect.x(), rect.y(), rect.width(), rect.height())
}

pub(super) fn sk_rect(rect: Rect) -> SkRect {
    SkRect::from_xywh(rect.x, rect.y, rect.width, rect.height)
}

pub(super) fn sk_matrix(value: Affine) -> Matrix {
    Matrix::new_all(
        value.xx, value.xy, value.dx, value.yx, value.yy, value.dy, 0.0, 0.0, 1.0,
    )
}

thread_local! {
    /// `ColorSpace::new_srgb()` crosses the FFI boundary to hand back what is a
    /// process-wide singleton on the Skia side. Every paint built per draw call
    /// used to pay for that round trip.
    static SRGB: ColorSpace = ColorSpace::new_srgb();
}

pub(super) fn srgb() -> ColorSpace {
    SRGB.with(Clone::clone)
}
