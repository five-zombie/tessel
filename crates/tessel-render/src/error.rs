use thiserror::Error;

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("no compatible GPU or software adapter found")]
    NoGpuAdapter,

    #[error("failed to request GPU device: {0}")]
    RequestDevice(#[from] wgpu::RequestDeviceError),

    #[error("failed to create window surface: {0}")]
    SurfaceCreation(#[from] wgpu::CreateSurfaceError),

    #[error("surface does not support any compatible texture format")]
    UnsupportedSurfaceFormat,

    #[error("failed to acquire surface texture: {0}")]
    SurfaceTexture(#[from] wgpu::SurfaceError),

    #[error("failed to encode viewport preview PNG: {0}")]
    PreviewEncode(String),
}
