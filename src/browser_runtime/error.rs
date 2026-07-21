use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrowserErrorCode {
    CorruptState,
    UnsupportedStateSchema,
    InvalidManifest,
    RuntimeIntegrityFailed,
    RuntimePathRejected,
}

impl BrowserErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CorruptState => "CORRUPT_STATE",
            Self::UnsupportedStateSchema => "UNSUPPORTED_STATE_SCHEMA",
            Self::InvalidManifest => "INVALID_MANIFEST",
            Self::RuntimeIntegrityFailed => "RUNTIME_INTEGRITY_FAILED",
            Self::RuntimePathRejected => "RUNTIME_PATH_REJECTED",
        }
    }
}

#[derive(Debug)]
pub struct BrowserError {
    code: BrowserErrorCode,
    message: String,
}

impl BrowserError {
    pub(crate) fn corrupt_state(message: impl Into<String>) -> Self {
        Self { code: BrowserErrorCode::CorruptState, message: message.into() }
    }

    pub(crate) fn unsupported_state_schema(message: impl Into<String>) -> Self {
        Self { code: BrowserErrorCode::UnsupportedStateSchema, message: message.into() }
    }

    pub(crate) fn invalid_manifest(message: impl Into<String>) -> Self {
        Self { code: BrowserErrorCode::InvalidManifest, message: message.into() }
    }

    pub(crate) fn runtime_integrity(message: impl Into<String>) -> Self {
        Self { code: BrowserErrorCode::RuntimeIntegrityFailed, message: message.into() }
    }

    pub(crate) fn runtime_path(message: impl Into<String>) -> Self {
        Self { code: BrowserErrorCode::RuntimePathRejected, message: message.into() }
    }

    pub const fn code(&self) -> BrowserErrorCode {
        self.code
    }
}

impl fmt::Display for BrowserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for BrowserError {}
