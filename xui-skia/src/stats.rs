/// Per-frame instrumentation for the Skia renderer.
///
/// Counters are reset by `begin_frame` and remain available after presentation.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SkiaFrameStats {
    pub frame_index: u64,
    pub root_damage_rects: usize,
    pub root_damage_area_sum: f32,
    pub primitive_draws: u64,
    pub layer_draws: u64,
    pub render_plans: u64,
    pub render_passes: u64,
    pub planned_transient_resources: u64,
    pub planned_transient_slots: u64,
    pub planned_transient_texels: u64,
    pub planned_peak_live_texels: u64,
    pub transient_surface_allocations: u64,
    pub transient_surface_reuses: u64,
    pub offscreen_surface_allocations: u64,
    pub image_snapshots: u64,
    pub backdrop_materializations: u64,
    pub backdrop_materializations_avoided: u64,
}
