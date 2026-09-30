//! Host configuration: theme, display scale, visibility, and the GPU device.

use crate::ui_runtime::UiRuntime;
use crate::ui_runtime::state::HostWorkFlags;
use xui_interface::{ComputedStyle, Theme};

impl UiRuntime {
    pub(crate) fn theme(&self) -> &Theme {
        &self.theme
    }

    pub(crate) fn set_theme(&mut self, theme: Theme) {
        if self.theme != theme {
            self.theme = theme;
            self.style_system
                .set_default_style(ComputedStyle::initial(&self.theme));
            self.mark_work(self.root, HostWorkFlags::RESTYLE_TREE);
        }
    }

    /// Whether the window is on screen. See [`Self::set_window_visible`].
    pub(crate) fn window_visible(&self) -> bool {
        self.window_visible
    }

    /// Hands the runtime the renderer's GPU device, so canvases with a GPU
    /// painter can draw on it.
    ///
    /// Called once by the renderer at startup. It must be the *renderer's*
    /// device: a texture from any other one cannot be composited without a
    /// copy, which is the point of this path.
    pub(crate) fn set_gpu_context(&mut self, context: crate::widgets::CanvasGpuContext) {
        self.gpu_context = Some(context);
    }

    /// Tells the runtime whether the window is on screen.
    ///
    /// Deliberately narrow: it gates the animation loop -- see
    /// [`Self::is_animating`] -- and nothing else. An ordinary change still
    /// repaints while hidden, so the window is already correct the instant it
    /// is revealed, and a platform that reports occlusion spuriously costs a
    /// few wasted frames rather than freezing the UI, which is the failure
    /// worth being asymmetric about.
    ///
    /// Nothing already in flight is touched. The set of canvases wanting
    /// another frame is only rewritten when a canvas compiles, and none do
    /// while hidden; a style animation keeps its elapsed time because the
    /// clock stops with it. So becoming visible resumes exactly where it
    /// stopped, however long that took.
    ///
    /// Drive it through [`crate::runtime::GuiRuntime::set_window_visible`],
    /// which stops the frame clock at the same moment.
    pub(crate) fn set_window_visible(&mut self, visible: bool) {
        self.window_visible = visible;
    }

    /// Physical pixels per logical pixel, for canvas painters.
    pub(crate) fn set_scale_factor(&mut self, scale_factor: f32) {
        if self.scale_factor == scale_factor {
            return;
        }
        self.scale_factor = scale_factor;
        // Hairline widths and pixel snapping are both derived from it, so every
        // drawing that used the old factor is now off by a subpixel.
        let canvases: Vec<_> = self.canvas_nodes.keys().collect();
        for id in canvases {
            self.ui_state.mark_canvas_dirty(id);
        }
    }
}
