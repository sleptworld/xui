use xui_interface::ImageKey;

#[derive(Debug, thiserror::Error)]
pub enum SkiaBackendError {
    #[error("failed to create a Skia surface of size {width}x{height}")]
    SurfaceAllocation { width: u32, height: u32 },
    #[error("failed to read pixels from the Skia surface")]
    PixelRead,
    #[error("invalid built frame: {0}")]
    InvalidFrame(String),
    #[error("failed to instantiate a layer render plan: {0}")]
    RenderPlan(#[from] xui_render_graph::PlanError),
    #[error("failed to compile the {effect} Skia runtime effect: {message}")]
    RuntimeEffect {
        effect: &'static str,
        message: String,
    },
    #[error("failed to create the {0} Skia runtime shader")]
    RuntimeShader(&'static str),
    #[error("failed to configure the {effect} Skia runtime effect: {message}")]
    RuntimeUniform {
        effect: &'static str,
        message: String,
    },
    #[error("render-program mask image {0:?} is not resident")]
    MissingMaskImage(ImageKey),
    #[error("render-program resource {0} is unavailable")]
    MissingResource(usize),
    #[error("softbuffer error: {0}")]
    SoftBuffer(#[from] softbuffer::SoftBufferError),
    #[error("failed to initialize Skia Metal: {0}")]
    MetalInitialization(String),
    #[error("failed to acquire or present a Metal drawable: {0}")]
    MetalPresentation(String),
    #[error("Font Data load error: {0}")]
    FontDataError(String),
}
