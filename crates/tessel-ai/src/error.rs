use serde_json::{json, Value};
use tessel_core::CoreError;
use tessel_io::IoError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AiError {
    #[error("core engine error: {0}")]
    Core(#[from] CoreError),

    #[error("export I/O error: {0}")]
    Io(#[from] IoError),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("HTTP provider error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("API key is required for this provider")]
    MissingApiKey,

    #[error("coordinates out of safe bounds (±256)")]
    OutOfBounds,

    #[error("batch limit exceeded (max 50,000 voxels per call)")]
    BatchLimitExceeded,

    #[error("unknown tool '{0}'")]
    UnknownTool(String),

    #[error("invalid tool arguments: {0}")]
    InvalidArguments(String),

    #[error("tool loop limit reached (max 8 iterations)")]
    ToolLoopLimitReached,

    #[error("provider returned error status {status}: {message}")]
    ProviderStatus { status: u16, message: String },
}

impl AiError {
    pub fn structured_code(&self) -> &'static str {
        match self {
            Self::Core(CoreError::NodeNotFound(_)) => "NODE_NOT_FOUND",
            Self::Core(CoreError::InvalidPaletteIndex) => "INVALID_PALETTE_INDEX",
            Self::Core(CoreError::OutOfBounds) | Self::OutOfBounds => "OUT_OF_BOUNDS",
            Self::Io(IoError::ExportPathDenied(_)) => "EXPORT_PATH_DENIED",
            Self::BatchLimitExceeded => "BATCH_LIMIT_EXCEEDED",
            Self::UnknownTool(_) => "UNKNOWN_TOOL",
            Self::InvalidArguments(_) => "INVALID_ARGUMENTS",
            Self::MissingApiKey => "MISSING_API_KEY",
            Self::ToolLoopLimitReached => "TOOL_LOOP_LIMIT_REACHED",
            _ => "INTERNAL_ERROR",
        }
    }

    pub fn hint(&self) -> &'static str {
        match self.structured_code() {
            "NODE_NOT_FOUND" => "Read tessel://scene/summary to list valid node IDs.",
            "INVALID_PALETTE_INDEX" => "Use a palette index between 1 and 255.",
            "OUT_OF_BOUNDS" => "Keep voxel coordinates within [-256, 256].",
            "BATCH_LIMIT_EXCEEDED" => "Split large fills into smaller boxes under 50,000 voxels.",
            "EXPORT_PATH_DENIED" => "Provide a relative filename without '..' traversal.",
            _ => "Inspect the message and retry with valid parameters.",
        }
    }

    pub fn to_structured_json(&self) -> Value {
        json!({
            "ok": false,
            "code": self.structured_code(),
            "message": self.to_string(),
            "hint": self.hint(),
        })
    }
}
