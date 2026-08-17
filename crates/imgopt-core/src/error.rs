use std::fmt;
use std::path::{Path, PathBuf};

/// The execution stage where an error occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipelineStage {
    Discovery,
    Hashing,
    ManifestLookup,
    FormatDetection,
    Decode,
    Analyze,
    CandidateGeneration,
    Encode,
    QualityEvaluation,
    OutputValidation,
    StorageUpload,
    StorageVerify,
    ManifestCommit,
    Config,
    ResourceCheck,
}

impl fmt::Display for PipelineStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Discovery => "discovery",
            Self::Hashing => "hashing",
            Self::ManifestLookup => "manifest_lookup",
            Self::FormatDetection => "format_detection",
            Self::Decode => "decode",
            Self::Analyze => "analyze",
            Self::CandidateGeneration => "candidate_generation",
            Self::Encode => "encode",
            Self::QualityEvaluation => "quality_evaluation",
            Self::OutputValidation => "output_validation",
            Self::StorageUpload => "storage_upload",
            Self::StorageVerify => "storage_verify",
            Self::ManifestCommit => "manifest_commit",
            Self::Config => "config",
            Self::ResourceCheck => "resource_check",
        };
        write!(f, "{name}")
    }
}

/// Category of storage-specific errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageErrorKind {
    AuthenticationFailed,
    NotFound,
    Timeout,
    ServerSide5xx { status_code: u16 },
    Network,
    Other,
}

impl fmt::Display for StorageErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AuthenticationFailed => write!(f, "authentication failed"),
            Self::NotFound => write!(f, "object not found"),
            Self::Timeout => write!(f, "request timeout"),
            Self::ServerSide5xx { status_code } => write!(f, "server error ({status_code})"),
            Self::Network => write!(f, "network error"),
            Self::Other => write!(f, "storage error"),
        }
    }
}

/// Detailed error classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidInput(String),
    UnsupportedFormat(String),
    Configuration(String),
    ResourceLimitExceeded(String),
    Decode(String),
    Encode(String),
    Quality(String),
    Storage {
        kind: StorageErrorKind,
        message: String,
    },
    Persistence(String),
    Io(String),
    Other(String),
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(msg) => write!(f, "invalid input: {msg}"),
            Self::UnsupportedFormat(msg) => write!(f, "unsupported format: {msg}"),
            Self::Configuration(msg) => write!(f, "configuration error: {msg}"),
            Self::ResourceLimitExceeded(msg) => write!(f, "resource limit exceeded: {msg}"),
            Self::Decode(msg) => write!(f, "decode error: {msg}"),
            Self::Encode(msg) => write!(f, "encode error: {msg}"),
            Self::Quality(msg) => write!(f, "quality evaluation error: {msg}"),
            Self::Storage { kind, message } => write!(f, "storage error ({kind}): {message}"),
            Self::Persistence(msg) => write!(f, "persistence error: {msg}"),
            Self::Io(msg) => write!(f, "I/O error: {msg}"),
            Self::Other(msg) => write!(f, "{msg}"),
        }
    }
}

/// Main typed error contract for Image Optimizer operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImgoptError {
    pub stage: PipelineStage,
    pub source_path: Option<PathBuf>,
    pub kind: ErrorKind,
}

impl ImgoptError {
    pub fn new(stage: PipelineStage, kind: ErrorKind) -> Self {
        Self {
            stage,
            source_path: None,
            kind,
        }
    }

    pub fn with_path<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.source_path = Some(path.as_ref().to_path_buf());
        self
    }

    /// Determines whether the error is transient and safe to retry.
    pub fn is_retryable(&self) -> bool {
        match &self.kind {
            ErrorKind::Storage { kind, .. } => match kind {
                StorageErrorKind::Timeout
                | StorageErrorKind::Network
                | StorageErrorKind::ServerSide5xx { .. } => true,
                StorageErrorKind::AuthenticationFailed
                | StorageErrorKind::NotFound
                | StorageErrorKind::Other => false,
            },
            ErrorKind::InvalidInput(_)
            | ErrorKind::UnsupportedFormat(_)
            | ErrorKind::Configuration(_)
            | ErrorKind::ResourceLimitExceeded(_)
            | ErrorKind::Decode(_)
            | ErrorKind::Encode(_)
            | ErrorKind::Quality(_)
            | ErrorKind::Persistence(_)
            | ErrorKind::Io(_)
            | ErrorKind::Other(_) => false,
        }
    }
}

impl fmt::Display for ImgoptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref path) = self.source_path {
            write!(
                f,
                "[{}] (path: {}) {}",
                self.stage,
                path.display(),
                self.kind
            )
        } else {
            write!(f, "[{}] {}", self.stage, self.kind)
        }
    }
}

impl std::error::Error for ImgoptError {}

impl From<imgopt_codecs::CodecError> for ImgoptError {
    fn from(e: imgopt_codecs::CodecError) -> Self {
        match e {
            imgopt_codecs::CodecError::DecodeError(msg) => {
                Self::new(PipelineStage::Decode, ErrorKind::Decode(msg))
            }
            imgopt_codecs::CodecError::EncodeError(msg) => {
                Self::new(PipelineStage::Encode, ErrorKind::Encode(msg))
            }
            imgopt_codecs::CodecError::UnsupportedFormat(msg) => Self::new(
                PipelineStage::FormatDetection,
                ErrorKind::UnsupportedFormat(msg),
            ),
            imgopt_codecs::CodecError::InvalidParameter(msg) => {
                Self::new(PipelineStage::Config, ErrorKind::Configuration(msg))
            }
            imgopt_codecs::CodecError::Io(msg) => {
                Self::new(PipelineStage::Discovery, ErrorKind::Io(msg))
            }
        }
    }
}

impl From<imgopt_quality::QualityError> for ImgoptError {
    fn from(e: imgopt_quality::QualityError) -> Self {
        Self::new(
            PipelineStage::QualityEvaluation,
            ErrorKind::Quality(e.to_string()),
        )
    }
}

pub type Result<T> = std::result::Result<T, ImgoptError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display_with_and_without_path() {
        let err_no_path = ImgoptError::new(
            PipelineStage::Decode,
            ErrorKind::Decode("corrupted JPEG header".into()),
        );
        assert_eq!(
            err_no_path.to_string(),
            "[decode] decode error: corrupted JPEG header"
        );

        let err_with_path = err_no_path.with_path("fixtures/test.jpg");
        assert_eq!(
            err_with_path.to_string(),
            "[decode] (path: fixtures/test.jpg) decode error: corrupted JPEG header"
        );
    }

    #[test]
    fn test_retryable_classification() {
        let timeout_err = ImgoptError::new(
            PipelineStage::StorageUpload,
            ErrorKind::Storage {
                kind: StorageErrorKind::Timeout,
                message: "timed out waiting for 200 OK".into(),
            },
        );
        assert!(timeout_err.is_retryable());

        let server_503_err = ImgoptError::new(
            PipelineStage::StorageUpload,
            ErrorKind::Storage {
                kind: StorageErrorKind::ServerSide5xx { status_code: 503 },
                message: "Service Unavailable".into(),
            },
        );
        assert!(server_503_err.is_retryable());

        let auth_err = ImgoptError::new(
            PipelineStage::StorageUpload,
            ErrorKind::Storage {
                kind: StorageErrorKind::AuthenticationFailed,
                message: "InvalidAccessKeyId".into(),
            },
        );
        assert!(!auth_err.is_retryable());

        let invalid_input_err = ImgoptError::new(
            PipelineStage::FormatDetection,
            ErrorKind::InvalidInput("empty file".into()),
        );
        assert!(!invalid_input_err.is_retryable());

        let config_err = ImgoptError::new(
            PipelineStage::Config,
            ErrorKind::Configuration("missing quality target".into()),
        );
        assert!(!config_err.is_retryable());
    }

    #[test]
    fn test_typed_matching() {
        let err = ImgoptError::new(
            PipelineStage::QualityEvaluation,
            ErrorKind::Quality("target SSIM not reachable".into()),
        );

        match err.kind {
            ErrorKind::Quality(msg) => assert_eq!(msg, "target SSIM not reachable"),
            _ => panic!("Expected Quality error"),
        }
    }
}
