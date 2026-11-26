use thiserror::Error;

#[derive(Debug, Error)]
pub enum IoError {
    #[error("I/O error: {0}")]
    Fs(#[from] std::io::Error),

    #[error("image encoding error: {0}")]
    Image(#[from] image::ImageError),

    #[error("GIF encoding error: {0}")]
    Gif(#[from] gif::EncodingError),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("ZIP archive error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("canvas or scene has no geometry to export")]
    EmptyCanvas,

    #[error("invalid image dimensions ({0}x{1})")]
    InvalidDimensions(u32, u32),

    #[error("invalid file format: {0}")]
    InvalidFormat(String),

    #[error("unsupported project schema version {0}; please update Tessel")]
    UnsupportedSchemaVersion(u32),

    #[error("export path denied: {0}")]
    ExportPathDenied(String),

    #[error("payload exceeds safety limit: {0}")]
    PayloadTooLarge(String),
}
