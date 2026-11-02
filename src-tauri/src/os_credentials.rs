//! The desktop's OS credential-store adapter.
//!
//! Provider and MCP credentials use different keychain services and different
//! domain types, but they must have the same access semantics. In particular,
//! `keyring::Error::NoEntry` means that one opaque reference is missing; it does
//! not mean that the OS credential store is unavailable. Keeping this mapping in
//! one place prevents the two callers from drifting apart.

use std::fmt::Display;
use std::sync::OnceLock;
use std::time::Duration;

use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::warn;

const KEYCHAIN_OPERATION_TIMEOUT: Duration = Duration::from_secs(10);

// macOS can attach an item's access prompt to the old development binary
// signature. Opening that item from a newer unsigned development binary can
// block in Security.framework. Debug builds therefore use a stable, separate
// namespace and ask the user to save the credential once again. Release
// bundles retain the original namespace so signed upgrades remain compatible
// with existing credentials.
#[cfg(debug_assertions)]
pub const PROVIDER_SERVICE: &str = "com.dgos.app.providers.dev";
#[cfg(not(debug_assertions))]
pub const PROVIDER_SERVICE: &str = "com.dgos.app.providers";
#[cfg(debug_assertions)]
pub const MCP_SERVICE: &str = "com.dgos.app.mcp.dev";
#[cfg(not(debug_assertions))]
pub const MCP_SERVICE: &str = "com.dgos.app.mcp";

// Security.framework is process-safe, but serializing calls avoids several
// simultaneous access prompts when startup recovery, an MCP monitor and a UI
// command all touch the login keychain at once.
static KEYCHAIN_LOCK: OnceLock<Arc<Semaphore>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialStoreFailure {
    Missing,
    Unavailable,
}

impl CredentialStoreFailure {
    pub fn classify(error: &keyring::Error) -> Self {
        match error {
            keyring::Error::NoEntry => Self::Missing,
            _ => Self::Unavailable,
        }
    }
}

impl Display for CredentialStoreFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => f.write_str("credential entry is missing"),
            Self::Unavailable => f.write_str("OS credential store is unavailable"),
        }
    }
}

/// Run a blocking keyring operation without allowing a locked/unresponsive
/// native backend to hold an application command forever. The operation keeps
/// running in its blocking worker after a timeout, but the caller is released;
/// no secret is included in the diagnostic.
pub async fn run<T, F>(operation: &'static str, task: F) -> Result<T, CredentialStoreFailure>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, keyring::Error> + Send + 'static,
{
    run_with_timeout(operation, KEYCHAIN_OPERATION_TIMEOUT, task).await
}

async fn run_with_timeout<T, F>(
    operation: &'static str,
    timeout: Duration,
    task: F,
) -> Result<T, CredentialStoreFailure>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, keyring::Error> + Send + 'static,
{
    let lock = KEYCHAIN_LOCK
        .get_or_init(|| Arc::new(Semaphore::new(1)))
        .clone();
    let permit = match tokio::time::timeout(timeout, lock.acquire_owned()).await {
        Ok(Ok(permit)) => permit,
        Ok(Err(_)) | Err(_) => {
            warn!(operation, "OS credential store is busy");
            return Err(CredentialStoreFailure::Unavailable);
        }
    };
    let result = tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || {
            // Keep the permit in the blocking worker: when a native call times
            // out, its worker may still be unwinding, so another call must not
            // enter Security.framework until that worker releases the permit.
            let _permit = permit;
            task()
        }),
    )
    .await;

    match result {
        Ok(Ok(Ok(value))) => Ok(value),
        Ok(Ok(Err(error))) => {
            let failure = CredentialStoreFailure::classify(&error);
            if failure == CredentialStoreFailure::Unavailable {
                warn!(operation, error = %error, "OS credential store operation failed");
            }
            Err(failure)
        }
        Ok(Err(error)) => {
            warn!(operation, error = %error, "OS credential store worker failed");
            Err(CredentialStoreFailure::Unavailable)
        }
        Err(_) => {
            warn!(operation, "OS credential store operation timed out");
            Err(CredentialStoreFailure::Unavailable)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CredentialStoreFailure;
    use std::time::Duration;

    #[test]
    fn only_no_entry_is_a_missing_credential() {
        assert_eq!(
            CredentialStoreFailure::classify(&keyring::Error::NoEntry),
            CredentialStoreFailure::Missing
        );
    }

    #[tokio::test]
    async fn a_blocked_backend_is_bounded() {
        let started = std::time::Instant::now();
        let result = super::run_with_timeout("test.blocked", Duration::from_millis(20), || {
            std::thread::sleep(Duration::from_millis(100));
            Ok::<_, keyring::Error>(())
        })
        .await;
        assert_eq!(result, Err(CredentialStoreFailure::Unavailable));
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
