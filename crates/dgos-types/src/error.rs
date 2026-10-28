use thiserror::Error;

#[derive(Error, Debug)]
pub enum DgosError {
    #[error("Host permission denied")]
    PermissionDenied,

    // Provider errors (1001xxx)
    #[error("Provider not found: {0}")]
    ProviderNotFound(String),

    #[error("Invalid API key")]
    InvalidApiKey,

    #[error("Provider credential is unavailable")]
    CredentialUnavailable,

    #[error("A new credential is required for this provider endpoint")]
    CredentialRequired,

    #[error("Provider configuration or model refresh changed; refresh and review before retrying")]
    ProviderConfigurationChanged,

    #[error("Provider unavailable: {0}")]
    ProviderUnavailable(String),

    #[error("Model not found: {0}")]
    ModelNotFound(String),

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("Request timeout")]
    Timeout,

    #[error("Stream interrupted")]
    StreamInterrupted,

    #[error("Max retries exceeded")]
    MaxRetriesExceeded,

    // Network errors
    #[error("Network error: {0}")]
    NetworkError(String),

    // HTTP errors
    #[error("HTTP error: {0}")]
    HttpError(u16),

    // Database errors
    #[error("Database error: {0}")]
    DatabaseError(String),

    // JSON parsing errors
    #[error("JSON parse error: {0}")]
    JsonError(String),

    // Generic error
    #[error("Internal error: {0}")]
    InternalError(String),
}

impl DgosError {
    pub fn error_code(&self) -> u32 {
        match self {
            DgosError::PermissionDenied => 1004004,
            DgosError::ProviderNotFound(_) => 1001001,
            DgosError::InvalidApiKey => 1001002,
            DgosError::CredentialUnavailable => 1001015,
            DgosError::CredentialRequired => 1001016,
            DgosError::ProviderConfigurationChanged => 1001012,
            DgosError::ProviderUnavailable(_) => 1001003,
            DgosError::ModelNotFound(_) => 1001004,
            DgosError::RateLimitExceeded => 1001005,
            DgosError::Timeout => 1001006,
            DgosError::StreamInterrupted => 1001007,
            DgosError::MaxRetriesExceeded => 1001008,
            _ => 1000000,
        }
    }
}

// Convert from reqwest::Error
impl From<reqwest::Error> for DgosError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            DgosError::Timeout
        } else if err.is_connect() {
            DgosError::NetworkError(err.to_string())
        } else if let Some(status) = err.status() {
            match status.as_u16() {
                401 => DgosError::InvalidApiKey,
                429 => DgosError::RateLimitExceeded,
                _ => DgosError::HttpError(status.as_u16()),
            }
        } else {
            DgosError::NetworkError(err.to_string())
        }
    }
}

impl From<serde_json::Error> for DgosError {
    fn from(err: serde_json::Error) -> Self {
        DgosError::JsonError(err.to_string())
    }
}

pub type DgosResult<T> = Result<T, DgosError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_codes() {
        assert_eq!(
            DgosError::ProviderNotFound("test".to_string()).error_code(),
            1001001
        );
        assert_eq!(DgosError::InvalidApiKey.error_code(), 1001002);
        assert_eq!(DgosError::RateLimitExceeded.error_code(), 1001005);
        assert_eq!(DgosError::Timeout.error_code(), 1001006);
        assert_eq!(DgosError::MaxRetriesExceeded.error_code(), 1001008);
    }
}
