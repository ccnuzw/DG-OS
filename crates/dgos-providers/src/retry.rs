use dgos_types::{DgosError, DgosResult, RetryConfig};
use std::future::Future;
use std::time::Duration;
use tokio::time::sleep;
use tracing::debug;

/// A failed attempt plus what the upstream said about retrying.
#[derive(Debug)]
pub struct AttemptError {
    pub error: DgosError,
    /// Parsed `retry-after` header; the policy caps it at `max_delay_ms`.
    pub retry_after: Option<Duration>,
    /// Never retry, whatever the error class (e.g. a stream that already went idle).
    pub terminal: bool,
}

impl AttemptError {
    pub fn new(error: DgosError) -> Self {
        Self {
            error,
            retry_after: None,
            terminal: false,
        }
    }

    pub fn terminal(error: DgosError) -> Self {
        Self {
            terminal: true,
            ..Self::new(error)
        }
    }

    pub fn with_retry_after(mut self, retry_after: Option<Duration>) -> Self {
        self.retry_after = retry_after;
        self
    }
}

impl From<DgosError> for AttemptError {
    fn from(error: DgosError) -> Self {
        Self::new(error)
    }
}

/// B01-55: only transient classes are retried. Everything else (bad key, unknown model, rejected
/// parameters, quota, permission, malformed responses, ...) fails at once with its own code.
pub fn is_retryable(error: &DgosError) -> bool {
    match error {
        DgosError::Timeout
        | DgosError::RateLimitExceeded
        | DgosError::UpstreamServerError
        | DgosError::NetworkError(_) => true,
        DgosError::HttpError(status) => matches!(status, 408 | 429 | 500..=599),
        _ => false,
    }
}

/// Retry policy for handling transient failures
pub struct RetryPolicy {
    config: RetryConfig,
}

impl RetryPolicy {
    pub fn new(config: RetryConfig) -> Self {
        Self { config }
    }

    /// Execute a function with retry logic
    pub async fn execute<F, Fut, T>(&self, mut f: F) -> DgosResult<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = DgosResult<T>>,
    {
        self.execute_hinted(|| {
            let attempt = f();
            async move { attempt.await.map_err(AttemptError::from) }
        })
        .await
    }

    /// Like [`execute`](Self::execute), for attempts that can report `retry-after` or mark a
    /// failure as final.
    ///
    /// A non-retryable error is returned unchanged. `MaxRetriesExceeded` is returned only after at
    /// least one retry has happened and also failed; with `max_retries = 0` the caller keeps the
    /// original code (§8.1).
    pub async fn execute_hinted<F, Fut, T>(&self, mut f: F) -> DgosResult<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, AttemptError>>,
    {
        let mut retries = 0u32;
        loop {
            let failure = match f().await {
                Ok(value) => return Ok(value),
                Err(failure) => failure,
            };
            if failure.terminal || !is_retryable(&failure.error) {
                return Err(failure.error);
            }
            if retries >= self.config.max_retries {
                debug!(
                    "Max retries ({}) exceeded, giving up",
                    self.config.max_retries
                );
                return Err(if retries > 0 {
                    DgosError::MaxRetriesExceeded
                } else {
                    failure.error
                });
            }
            let delay = self.delay_for(retries, failure.retry_after);
            debug!(
                "Attempt {} failed with code {}, retrying in {:?}",
                retries + 1,
                failure.error.error_code(),
                delay
            );
            sleep(delay).await;
            retries += 1;
        }
    }

    /// `min(retry-after or backoff, max_delay)` (B01-55).
    fn delay_for(&self, attempt: u32, retry_after: Option<Duration>) -> Duration {
        let wanted =
            retry_after.unwrap_or_else(|| Duration::from_millis(self.calculate_backoff(attempt)));
        wanted.min(Duration::from_millis(self.config.max_delay_ms))
    }

    /// Calculate exponential backoff delay
    fn calculate_backoff(&self, attempt: u32) -> u64 {
        let factor = 1u64.checked_shl(attempt).unwrap_or(u64::MAX);
        self.config
            .initial_delay_ms
            .saturating_mul(factor)
            .min(self.config.max_delay_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
    use std::sync::Arc;

    fn fast(max_retries: u32) -> RetryPolicy {
        RetryPolicy::new(RetryConfig {
            max_retries,
            initial_delay_ms: 1,
            max_delay_ms: 5,
        })
    }

    #[test]
    fn test_calculate_backoff() {
        let policy = RetryPolicy::new(RetryConfig {
            max_retries: 3,
            initial_delay_ms: 1000,
            max_delay_ms: 8000,
        });

        assert_eq!(policy.calculate_backoff(0), 1000); // 1s
        assert_eq!(policy.calculate_backoff(1), 2000); // 2s
        assert_eq!(policy.calculate_backoff(2), 4000); // 4s
        assert_eq!(policy.calculate_backoff(3), 8000); // 8s (capped)
        assert_eq!(policy.calculate_backoff(4), 8000); // 8s (capped)
        assert_eq!(policy.calculate_backoff(200), 8000); // no overflow
    }

    #[tokio::test]
    async fn test_retry_success_on_first_attempt() {
        let policy = RetryPolicy::new(RetryConfig::default());
        let call_count = Arc::new(AtomicUsize::new(0));
        let call_count_clone = call_count.clone();

        let result = policy
            .execute(|| {
                let count = call_count_clone.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, DgosError>(42)
                }
            })
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retry_success_after_failures() {
        let policy = RetryPolicy::new(RetryConfig {
            max_retries: 2,
            initial_delay_ms: 10,
            max_delay_ms: 100,
        });
        let call_count = Arc::new(AtomicUsize::new(0));
        let call_count_clone = call_count.clone();

        let result = policy
            .execute(|| {
                let count = call_count_clone.clone();
                async move {
                    let current = count.fetch_add(1, Ordering::SeqCst) + 1;
                    if current < 2 {
                        Err(DgosError::NetworkError("test".to_string()))
                    } else {
                        Ok::<_, DgosError>(42)
                    }
                }
            })
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_retry_exhausted() {
        let policy = RetryPolicy::new(RetryConfig {
            max_retries: 2,
            initial_delay_ms: 10,
            max_delay_ms: 100,
        });
        let call_count = Arc::new(AtomicUsize::new(0));
        let call_count_clone = call_count.clone();

        let result = policy
            .execute(|| {
                let count = call_count_clone.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Err::<i32, _>(DgosError::NetworkError("test".to_string()))
                }
            })
            .await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DgosError::MaxRetriesExceeded));
        assert_eq!(call_count.load(Ordering::SeqCst), 3); // 1 + 2 retries
    }

    #[tokio::test]
    async fn test_no_retry_on_invalid_api_key() {
        let policy = RetryPolicy::new(RetryConfig::default());
        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let result = policy
            .execute(|| {
                let count = call_count_clone.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Err::<i32, _>(DgosError::InvalidApiKey)
                }
            })
            .await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DgosError::InvalidApiKey));
        assert_eq!(call_count.load(Ordering::SeqCst), 1); // No retries
    }

    async fn failures_seen(policy: &RetryPolicy, make: fn() -> DgosError) -> (DgosError, u32) {
        let attempts = Arc::new(AtomicU32::new(0));
        let counter = attempts.clone();
        let error = policy
            .execute(|| {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Err::<(), _>(make())
                }
            })
            .await
            .unwrap_err();
        (error, attempts.load(Ordering::SeqCst))
    }

    #[tokio::test]
    async fn retry_classification_never_retries_permanent_failures_and_keeps_their_code() {
        let policy = fast(3);
        let cases: [(fn() -> DgosError, u32); 13] = [
            (|| DgosError::ProviderRequestRejected, 1001017),
            (|| DgosError::ContextLengthExceeded, 1001018),
            (|| DgosError::ProviderPermissionDenied, 1001019),
            (|| DgosError::ProviderQuotaExhausted, 1001024),
            (|| DgosError::ProviderResponseInvalid, 1001025),
            (|| DgosError::ModelNotFound("m".into()), 1001004),
            (|| DgosError::InvalidApiKey, 1001002),
            (|| DgosError::InsecureEndpoint, 1001023),
            (|| DgosError::StreamInterrupted, 1001007),
            (|| DgosError::JsonError("bad".into()), 1000000),
            (|| DgosError::InternalError("bad".into()), 1000000),
            (|| DgosError::HttpError(400), 1000000),
            (|| DgosError::HttpError(404), 1000000),
        ];
        for (make, code) in cases {
            let (error, attempts) = failures_seen(&policy, make).await;
            assert_eq!(attempts, 1, "code {code} must not be retried");
            assert_eq!(error.error_code(), code, "must not become 1001008");
        }
    }

    #[tokio::test]
    async fn retry_classification_retries_transient_failures_then_reports_exhaustion() {
        let policy = fast(3);
        for make in [
            (|| DgosError::Timeout) as fn() -> DgosError,
            || DgosError::RateLimitExceeded,
            || DgosError::UpstreamServerError,
            || DgosError::NetworkError("down".into()),
            || DgosError::HttpError(503),
        ] {
            let (error, attempts) = failures_seen(&policy, make).await;
            assert_eq!(attempts, 4, "1 call + 3 retries");
            assert_eq!(error.error_code(), 1001008);
        }
    }

    #[tokio::test]
    async fn retry_classification_without_configured_retries_keeps_the_original_code() {
        let policy = fast(0);
        let (error, attempts) = failures_seen(&policy, || DgosError::UpstreamServerError).await;
        assert_eq!(attempts, 1);
        assert_eq!(error.error_code(), 1001014);
        let (error, _) = failures_seen(&policy, || DgosError::RateLimitExceeded).await;
        assert_eq!(error.error_code(), 1001005);
    }

    #[tokio::test]
    async fn retry_classification_terminal_failures_are_not_retried_even_when_transient() {
        let attempts = Arc::new(AtomicU32::new(0));
        let counter = attempts.clone();
        let error = fast(3)
            .execute_hinted(|| {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Err::<(), _>(AttemptError::terminal(DgosError::Timeout))
                }
            })
            .await
            .unwrap_err();
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert_eq!(error.error_code(), 1001006);
    }

    #[test]
    fn retry_classification_waits_for_the_smaller_of_retry_after_and_max_delay() {
        let policy = RetryPolicy::new(RetryConfig {
            max_retries: 3,
            initial_delay_ms: 1000,
            max_delay_ms: 8000,
        });
        // Header shorter than the cap: honoured.
        assert_eq!(
            policy.delay_for(0, Some(Duration::from_secs(2))),
            Duration::from_secs(2)
        );
        // Header longer than the cap: capped.
        assert_eq!(
            policy.delay_for(0, Some(Duration::from_secs(600))),
            Duration::from_secs(8)
        );
        // No header: exponential backoff, also capped.
        assert_eq!(policy.delay_for(1, None), Duration::from_secs(2));
        assert_eq!(policy.delay_for(9, None), Duration::from_secs(8));
    }
}
