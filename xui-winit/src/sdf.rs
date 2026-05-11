pub const SDF_SNIPPETS_WGSL: &str = include_str!("shaders/sdf_snippets.wgsl");
pub const UI_SDF_SHADER_WGSL: &str = include_str!("shaders/ui_sdf.wgsl");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_basic_ui_sdf_primitives() {
        assert!(SDF_SNIPPETS_WGSL.contains("fn sdf_rect"));
        assert!(SDF_SNIPPETS_WGSL.contains("fn sdf_rounded_rect"));
        assert!(SDF_SNIPPETS_WGSL.contains("fn sdf_circle"));
        assert!(SDF_SNIPPETS_WGSL.contains("fn sdf_fill_alpha"));
        assert!(SDF_SNIPPETS_WGSL.contains("fn sdf_stroke_alpha"));
    }

    #[test]
    fn full_shader_reuses_sdf_snippets() {
        assert!(UI_SDF_SHADER_WGSL.contains("fn vs_main"));
        assert!(UI_SDF_SHADER_WGSL.contains("fn fs_main"));
        assert!(UI_SDF_SHADER_WGSL.contains("sdf_rounded_rect"));
        assert!(UI_SDF_SHADER_WGSL.contains("sdf_circle"));
    }
}
