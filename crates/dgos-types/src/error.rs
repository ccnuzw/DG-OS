use crate::module_operation::ModuleOperationErrorKind;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DgosError {
    #[error("Host permission denied")]
    PermissionDenied,

    // Appearance preference errors (1005xxx, D065); fixed messages that never echo input.
    #[error("Unknown theme")]
    ThemeNotFound,

    #[error("Unsupported interface density")]
    InvalidDensity,

    #[error("Invalid appearance preferences patch")]
    InvalidAppearancePatch,

    // Module lifecycle errors (1004xxx); the message is a fixed string.
    #[error("{}", .0.message())]
    ModuleOperation(ModuleOperationErrorKind),

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

    // V1-FR-001 §8.1 (D063). Fixed messages: upstream bodies, keys and user input are never echoed.
    #[error("Provider upstream server error")]
    UpstreamServerError,

    #[error("Provider rejected the request parameters")]
    ProviderRequestRejected,

    #[error("Input exceeds the model context window")]
    ContextLengthExceeded,

    #[error("Provider denied permission for this key or region")]
    ProviderPermissionDenied,

    #[error("Request is not supported by the provider protocol")]
    RequestNotSupported,

    #[error("Provider output is empty or does not match the requested schema")]
    StructuredOutputInvalid,

    #[error("Unsupported provider type")]
    UnsupportedProviderType,

    #[error("Endpoint must use https unless it is a local or private address")]
    InsecureEndpoint,

    #[error("Provider quota exhausted")]
    ProviderQuotaExhausted,

    #[error("Provider response is not the expected format")]
    ProviderResponseInvalid,

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
            DgosError::ThemeNotFound => 1005001,
            DgosError::InvalidAppearancePatch => 1005002,
            DgosError::InvalidDensity => 1005006,
            DgosError::ModuleOperation(kind) => kind.code(),
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
            DgosError::UpstreamServerError => 1001014,
            DgosError::ProviderRequestRejected => 1001017,
            DgosError::ContextLengthExceeded => 1001018,
            DgosError::ProviderPermissionDenied => 1001019,
            DgosError::RequestNotSupported => 1001020,
            DgosError::StructuredOutputInvalid => 1001021,
            DgosError::UnsupportedProviderType => 1001022,
            DgosError::InsecureEndpoint => 1001023,
            DgosError::ProviderQuotaExhausted => 1001024,
            DgosError::ProviderResponseInvalid => 1001025,
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

    #[test]
    fn fr001_extension_codes_match_the_registry_and_use_fixed_messages() {
        let cases = [
            (DgosError::UpstreamServerError, 1001014),
            (DgosError::ProviderRequestRejected, 1001017),
            (DgosError::ContextLengthExceeded, 1001018),
            (DgosError::ProviderPermissionDenied, 1001019),
            (DgosError::RequestNotSupported, 1001020),
            (DgosError::StructuredOutputInvalid, 1001021),
            (DgosError::UnsupportedProviderType, 1001022),
            (DgosError::InsecureEndpoint, 1001023),
            (DgosError::ProviderQuotaExhausted, 1001024),
            (DgosError::ProviderResponseInvalid, 1001025),
        ];
        for (error, code) in cases {
            assert_eq!(error.error_code(), code);
            // Fixed text: nothing a caller or an upstream body could inject.
            assert!(!error.to_string().contains(':'), "{error}");
        }
    }

    #[test]
    fn module_operation_errors_expose_their_code_and_a_fixed_message() {
        let error = DgosError::ModuleOperation(ModuleOperationErrorKind::StopFailed);
        assert_eq!(error.error_code(), 1004007);
        assert_eq!(
            error.to_string(),
            ModuleOperationErrorKind::StopFailed.message()
        );
        // PermissionDenied keeps its historical wire code.
        assert_eq!(DgosError::PermissionDenied.error_code(), 1004004);
    }
}
