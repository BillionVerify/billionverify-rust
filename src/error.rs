use std::fmt;

/// Details shared by every API error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorInfo {
    /// Human-readable message.
    pub message: String,
    /// Machine-readable code (`error.code` of the response, `UNKNOWN_ERROR` when absent).
    pub code: String,
    /// HTTP status code, `0` for network errors.
    pub status_code: u16,
    /// Extra context sent by the API, when present.
    pub details: Option<String>,
}

impl ErrorInfo {
    pub(crate) fn new(message: impl Into<String>, code: &str, status_code: u16) -> Self {
        Self {
            message: message.into(),
            code: code.to_string(),
            status_code,
            details: None,
        }
    }
}

impl fmt::Display for ErrorInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/// Every error the client can return.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The API key is missing or invalid (HTTP 401).
    #[error("{0}")]
    Authentication(ErrorInfo),
    /// The rate limit was exceeded (HTTP 429) and retries ran out.
    #[error("{info}")]
    RateLimit {
        /// Error details.
        info: ErrorInfo,
        /// Seconds to wait, from the `Retry-After` header (`0` when absent).
        retry_after: u64,
    },
    /// The request was rejected as invalid (HTTP 400), or failed local validation.
    #[error("{0}")]
    Validation(ErrorInfo),
    /// The account has too few credits (HTTP 402).
    #[error("{0}")]
    InsufficientCredits(ErrorInfo),
    /// The resource does not exist (HTTP 404).
    #[error("{0}")]
    NotFound(ErrorInfo),
    /// The request or a wait timed out.
    #[error("{0}")]
    Timeout(ErrorInfo),
    /// Any other API error (409, 500, ...), or a response that could not be read.
    #[error("{0}")]
    Api(ErrorInfo),
    /// The request never got a response.
    #[error("{0}")]
    Network(ErrorInfo),
}

impl Error {
    /// The shared error details.
    pub fn info(&self) -> &ErrorInfo {
        match self {
            Error::Authentication(i)
            | Error::Validation(i)
            | Error::InsufficientCredits(i)
            | Error::NotFound(i)
            | Error::Timeout(i)
            | Error::Api(i)
            | Error::Network(i) => i,
            Error::RateLimit { info, .. } => info,
        }
    }

    /// Human-readable message.
    pub fn message(&self) -> &str {
        &self.info().message
    }

    /// Machine-readable code.
    pub fn code(&self) -> &str {
        &self.info().code
    }

    /// HTTP status code, `0` for network errors.
    pub fn status_code(&self) -> u16 {
        self.info().status_code
    }

    /// Extra context sent by the API.
    pub fn details(&self) -> Option<&str> {
        self.info().details.as_deref()
    }

    pub(crate) fn validation(message: impl Into<String>) -> Self {
        Error::Validation(ErrorInfo::new(message, "INVALID_REQUEST", 400))
    }

    pub(crate) fn timeout(message: impl Into<String>) -> Self {
        Error::Timeout(ErrorInfo::new(message, "TIMEOUT", 504))
    }

    pub(crate) fn authentication(message: impl Into<String>) -> Self {
        Error::Authentication(ErrorInfo::new(message, "INVALID_API_KEY", 401))
    }

    pub(crate) fn invalid_response(message: impl Into<String>, status: u16) -> Self {
        Error::Api(ErrorInfo::new(message, "INVALID_RESPONSE", status))
    }
}

/// Result alias used by every client method.
pub type Result<T> = std::result::Result<T, Error>;
