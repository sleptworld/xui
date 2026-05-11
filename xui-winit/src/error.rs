#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("WGPU error: {0}")]
    Wgpu(#[from] wgpu::Error),
    #[error("Winit error: {0}")]
    Winit(#[from] winit::error::OsError),
    #[error("Other error: {0}")]
    Other(String),
}
