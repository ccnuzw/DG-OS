use dgos_types::{DgosError, DgosResult, RetryConfig};
use std::time::Duration;
use tokio::time::sleep;
use tracing::debug;

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
        Fut: std::future::Future<Output = DgosResult<T>>,
    {
        let mut attempts = 0;
        let max_attempts = self.config.max_retries + 1;

        loop {
            attempts += 1;

            match f().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    // Don't retry on these errors
                    if matches!(
                        e,
                        DgosError::InvalidApiKey | DgosError::ProviderNotFound(_)
                    ) {
                        return Err(e);
                    }

                    // Check if we should retry
                    if attempts >= max_attempts {
                        debug!(
                            "Max retries ({}) exceeded, giving up",
                            self.config.max_retries
                        );
                        return Err(DgosError::MaxRetriesExceeded);
                    }

                    // Calculate backoff delay using exponential backoff
                    let delay_ms = self.calculate_backoff(attempts - 1);
                    debug!(
                        "Attempt {}/{} failed: {:?}, retrying in {}ms",
                        attempts, max_attempts, e, delay_ms
                    );

                    sleep(Duration::from_millis(delay_ms)).await;
                }
            }
        }
    }

    /// Calculate exponential backoff delay
    fn calculate_backoff(&self, attempt: u32) -> u64 {
        let delay = self.config.initial_delay_ms * (2_u64.pow(attempt));
        delay.min(self.config.max_delay_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[tokio::test]
    async fn test_retry_success_on_first_attempt() {
        let policy = RetryPolicy::new(RetryConfig::default());
        let call_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let call_count_clone = call_count.clone();

        let result = policy
            .execute(|| {
                let count = call_count_clone.clone();
                async move {
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok::<_, DgosError>(42)
                }
            })
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
        assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retry_success_after_failures() {
        let policy = RetryPolicy::new(RetryConfig {
            max_retries: 2,
            initial_delay_ms: 10,
            max_delay_ms: 100,
        });
        let call_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let call_count_clone = call_count.clone();

        let result = policy
            .execute(|| {
                let count = call_count_clone.clone();
                async move {
                    let current = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
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
        assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_retry_exhausted() {
        let policy = RetryPolicy::new(RetryConfig {
            max_retries: 2,
            initial_delay_ms: 10,
            max_delay_ms: 100,
        });
        let call_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let call_count_clone = call_count.clone();

        let result = policy
            .execute(|| {
                let count = call_count_clone.clone();
                async move {
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Err::<i32, _>(DgosError::NetworkError("test".to_string()))
                }
            })
            .await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DgosError::MaxRetriesExceeded));
        assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 3); // 1 + 2 retries
    }

    #[tokio::test]
    async fn test_no_retry_on_invalid_api_key() {
        let policy = RetryPolicy::new(RetryConfig::default());
        let call_count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let result = policy
            .execute(|| {
                let count = call_count_clone.clone();
                async move {
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Err::<i32, _>(DgosError::InvalidApiKey)
                }
            })
            .await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DgosError::InvalidApiKey));
        assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 1); // No retries
    }
}
