use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Database error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("State persistence error: {0}")]
    State(String),

    #[error("Archive error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("JSON serialization/deserialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Archive path traversal detected: {0}")]
    PathTraversal(String),

    #[error("Security threat detected: {0}")]
    SecurityThreat(String),

    #[error("Insufficient disk space")]
    DiskFull,

    #[error("Missing timestamp in JSON sidecar")]
    NoTimestamp,

    #[error("Invalid GPS coordinate")]
    InvalidGps,

    #[error("Invalid file path format: {0}")]
    InvalidFilePath(String),

    #[error("Invalid file status value")]
    InvalidStatus,

    #[error("Too many collisions resolving output filename")]
    TooManyCollisions,

    #[error("Operation cancelled by user")]
    Cancelled,

    #[error("ExifTool process launch failed: {0}")]
    ExifToolLaunchFailed(String),

    #[error("ExifTool process crashed or terminated unexpectedly")]
    ExifToolCrashed,

    #[error("ExifTool timeout exceeded")]
    Timeout,

    #[error("ExifTool binary not found at expected location")]
    ExifToolNotFound,

    #[error("Checksum mismatch for downloaded file")]
    ChecksumMismatch,

    #[error("Configuration error: {0}")]
    Config(String),
}

impl AppError {
    /// Classifies whether this error should abort the processing of a single file,
    /// but allow the overall processing run to continue with other files.
    pub fn is_file_recoverable(&self) -> bool {
        matches!(
            self,
            AppError::Io(_)
                | AppError::Json(_)
                | AppError::NoTimestamp
                | AppError::InvalidGps
                | AppError::Timeout
                | AppError::ExifToolCrashed
                | AppError::TooManyCollisions
        )
    }
}
