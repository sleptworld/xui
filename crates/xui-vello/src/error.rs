#[derive(Debug, thiserror::Error)]
pub enum VelloBackendError {
    #[error("failed to create the wgpu surface: {0}")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),
    #[error("no suitable wgpu adapter: {0}")]
    RequestAdapter(#[from] wgpu::RequestAdapterError),
    #[error("failed to open the wgpu device: {0}")]
    RequestDevice(#[from] wgpu::RequestDeviceError),
    #[error("the adapter cannot render Vello output: Rgba8Unorm lacks storage binding")]
    UnsupportedVectorTarget,
    #[error("the surface reports no usable configuration")]
    UnsupportedSurface,
    #[error("the surface texture could not be acquired")]
    AcquireSurface,
    #[error("no frame has been rendered yet")]
    NoFrame,
    #[error("reading pixels back failed: {0}")]
    Readback(String),
    #[error("the render plan cannot be run: {0}")]
    Plan(&'static str),
    #[error("vello: {0}")]
    Vello(#[from] vello::Error),
}
