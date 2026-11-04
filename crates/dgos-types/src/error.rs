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

    #[error("Appearance revision conflict")]
    AppearanceRevisionConflict,
    #[error("Appearance recovery is no longer valid")]
    AppearanceRecoveryInvalid,
    #[error("Appearance schema is not supported")]
    AppearanceSchemaUnsupported,

    #[error("Invalid language preference")]
    InvalidLanguagePreference,

    // Module lifecycle errors (1004xxx); the message is a fixed string.
    #[error("{}", .0.message())]
    ModuleOperation(ModuleOperationErrorKind),

    // Module capability call errors (D093); fixed messages that never echo input.
    #[error("Invalid module capability input")]
    ModuleCapabilityInputInvalid,

    #[error("Module storage quota exceeded")]
    ModuleStorageQuotaExceeded,

    #[error("Network target refused")]
    ModuleNetworkTargetRefused,

    #[error("Network response too large")]
    ModuleNetworkResponseTooLarge,

    /// The page cancelled a module capability call (D113, `1004028`).
    #[error("Module call cancelled")]
    ModuleCallCancelled,

    /// A failure of the capability itself (Skills `1002xxx`, MCP `1003xxx`),
    /// keeping its own code (D048, D093). The message is the capability's
    /// fixed text, never request or response content.
    #[error("{message}")]
    CapabilityFailed { code: u32, message: String },

    // Local account errors (1008xxx, D087/D090). Fixed messages: never echo the
    // username, password, hash or internal user id.
    #[error("Username already exists")]
    UsernameTaken,

    #[error("Password must be 8 to 128 characters")]
    PasswordRejected,

    #[error(
        "Username must be 1 to 32 characters without surrounding spaces or control characters"
    )]
    UsernameRejected,

    #[error("Wrong username or password")]
    SignInRejected,

    /// Signed out, switching or deleting an account, or a mode that does not
    /// allow the account operation (D090).
    #[error("No valid session for this operation")]
    SessionInvalid,

    /// The account deletion stopped part way; startup recovery continues it.
    #[error("Account deletion did not finish; it resumes on the next start")]
    AccountDeletionIncomplete,

    #[error("Too many failed sign-in attempts; try again later")]
    SignInThrottled { retry_after_seconds: u64 },

    /// The database file could not be opened or migrated at startup (D098
    /// item 9, D116). Nothing was repaired. `stage` is `open` or `migrate`;
    /// `reason` is a bounded line without the path; `path` is the file.
    #[error("The local database could not be opened")]
    DatabaseUnavailable {
        stage: String,
        reason: String,
        path: String,
    },

    // LAN collaboration (1010xxx, FR-010). Host command `set_lan_port` only (D107).
    #[error("LAN port must be 1024 to 65535 and not a browser-blocked port")]
    LanPortInvalid,

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
            DgosError::InvalidLanguagePreference => 1005002,
            DgosError::InvalidDensity => 1005006,
            DgosError::AppearanceRevisionConflict => 1005007,
            DgosError::AppearanceRecoveryInvalid => 1005008,
            DgosError::AppearanceSchemaUnsupported => 1005009,
            DgosError::ModuleOperation(kind) => kind.code(),
            DgosError::ModuleCapabilityInputInvalid => 1004024,
            DgosError::ModuleStorageQuotaExceeded => 1004025,
            DgosError::ModuleNetworkTargetRefused => 1004026,
            DgosError::ModuleNetworkResponseTooLarge => 1004027,
            DgosError::ModuleCallCancelled => 1004028,
            DgosError::CapabilityFailed { code, .. } => *code,
            DgosError::UsernameTaken => 1008001,
            DgosError::PasswordRejected => 1008002,
            DgosError::UsernameRejected => 1008003,
            DgosError::SignInRejected => 1008005,
            DgosError::SessionInvalid => 1008007,
            DgosError::AccountDeletionIncomplete => 1008009,
            DgosError::SignInThrottled { .. } => 1008011,
            DgosError::DatabaseUnavailable { .. } => 1008012,
            DgosError::LanPortInvalid => 1010012,
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

    /// Structured detail for the IPC error envelope. Only fields that are safe
    /// to show the renderer; never input values.
    pub fn detail(&self) -> Option<serde_json::Value> {
        match self {
            DgosError::SignInThrottled {
                retry_after_seconds,
            } => Some(serde_json::json!({ "retry_after_seconds": retry_after_seconds })),
            DgosError::DatabaseUnavailable {
                stage,
                reason,
                path,
            } => Some(serde_json::json!({ "stage": stage, "reason": reason, "path": path })),
            _ => None,
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

    #[test]
    fn account_codes_match_the_registry_and_only_throttling_and_unavailable_carry_detail() {
        let cases = [
            (DgosError::UsernameTaken, 1008001),
            (DgosError::PasswordRejected, 1008002),
            (DgosError::UsernameRejected, 1008003),
            (DgosError::SignInRejected, 1008005),
            (DgosError::SessionInvalid, 1008007),
            (DgosError::AccountDeletionIncomplete, 1008009),
        ];
        for (error, code) in cases {
            assert_eq!(error.error_code(), code);
            assert!(error.detail().is_none());
        }
        let throttled = DgosError::SignInThrottled {
            retry_after_seconds: 42,
        };
        assert_eq!(throttled.error_code(), 1008011);
        assert_eq!(
            throttled.detail(),
            Some(serde_json::json!({ "retry_after_seconds": 42 }))
        );
        assert!(!throttled.to_string().contains("42"));
        let unavailable = DgosError::DatabaseUnavailable {
            stage: "migrate".into(),
            reason: "Migration file is missing".into(),
            path: "/data/dgos.db".into(),
        };
        assert_eq!(unavailable.error_code(), 1008012);
        assert_eq!(
            unavailable.detail(),
            Some(serde_json::json!({
                "stage": "migrate",
                "reason": "Migration file is missing",
                "path": "/data/dgos.db"
            }))
        );
        // The message is fixed text; the reason and path travel only in `detail`.
        assert!(!unavailable.to_string().contains("dgos.db"));
    }
}
