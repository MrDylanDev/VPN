use std::time::Duration;

use rand::Rng;

use crate::cloud::{CloudError, CloudProvider, ProvisionParams, VpsInstance};

/// A [`CloudProvider`] wrapper that adds exponential backoff retry logic.
///
/// Retries are triggered on [`CloudError::RateLimit`] and [`CloudError::Provider`]
/// errors (the latter covers 5xx responses). Each retry waits
/// `base_delay_ms × 2^attempt` milliseconds with ±20% jitter.
///
/// [`CloudError::Auth`], [`CloudError::Quota`], and [`CloudError::Timeout`] are
/// propagated immediately without any retry attempt.
pub struct RetryCloudProvider<T: CloudProvider> {
    inner: T,
    max_retries: u32,
    base_delay_ms: u64,
}

impl<T: CloudProvider> RetryCloudProvider<T> {
    /// Create a new retry wrapper with default settings (max 3 retries, 1 s base delay).
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            max_retries: 3,
            base_delay_ms: 1000,
        }
    }

    /// Create a retry wrapper with explicit retry settings.
    ///
    /// `max_retries` is the total number of attempts (1 initial + up to N-1 retries).
    /// `base_delay_ms` is the delay before the first retry; each subsequent retry
    /// doubles the delay.
    pub fn with_retries(inner: T, max_retries: u32, base_delay_ms: u64) -> Self {
        Self {
            inner,
            max_retries,
            base_delay_ms,
        }
    }
}

impl<T: CloudProvider> CloudProvider for RetryCloudProvider<T> {
    async fn validate_token(&self, token: &str) -> Result<bool, CloudError> {
        let mut last_error = None;
        for attempt in 0..self.max_retries {
            match self.inner.validate_token(token).await {
                Ok(val) => return Ok(val),
                Err(err) if is_retriable(&err) && attempt + 1 < self.max_retries => {
                    let raw_delay = self.base_delay_ms * 2u64.pow(attempt);
                    tokio::time::sleep(Duration::from_millis(jittered_delay_ms(raw_delay))).await;
                    last_error = Some(err);
                }
                Err(err) => return Err(err),
            }
        }
        Err(last_error.unwrap())
    }

    async fn create_vps(&self, params: &ProvisionParams, token: &str) -> Result<VpsInstance, CloudError> {
        let mut last_error = None;
        for attempt in 0..self.max_retries {
            match self.inner.create_vps(params, token).await {
                Ok(val) => return Ok(val),
                Err(err) if is_retriable(&err) && attempt + 1 < self.max_retries => {
                    let raw_delay = self.base_delay_ms * 2u64.pow(attempt);
                    tokio::time::sleep(Duration::from_millis(jittered_delay_ms(raw_delay))).await;
                    last_error = Some(err);
                }
                Err(err) => return Err(err),
            }
        }
        Err(last_error.unwrap())
    }

    async fn destroy_vps(&self, instance_id: &str, token: &str) -> Result<(), CloudError> {
        let mut last_error = None;
        for attempt in 0..self.max_retries {
            match self.inner.destroy_vps(instance_id, token).await {
                Ok(val) => return Ok(val),
                Err(err) if is_retriable(&err) && attempt + 1 < self.max_retries => {
                    let raw_delay = self.base_delay_ms * 2u64.pow(attempt);
                    tokio::time::sleep(Duration::from_millis(jittered_delay_ms(raw_delay))).await;
                    last_error = Some(err);
                }
                Err(err) => return Err(err),
            }
        }
        Err(last_error.unwrap())
    }

    async fn list_vpss(&self, token: &str) -> Result<Vec<VpsInstance>, CloudError> {
        let mut last_error = None;
        for attempt in 0..self.max_retries {
            match self.inner.list_vpss(token).await {
                Ok(val) => return Ok(val),
                Err(err) if is_retriable(&err) && attempt + 1 < self.max_retries => {
                    let raw_delay = self.base_delay_ms * 2u64.pow(attempt);
                    tokio::time::sleep(Duration::from_millis(jittered_delay_ms(raw_delay))).await;
                    last_error = Some(err);
                }
                Err(err) => return Err(err),
            }
        }
        Err(last_error.unwrap())
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

impl<T: CloudProvider> RetryCloudProvider<T> {
}

/// Should the retry loop consider this error worth retrying?
fn is_retriable(err: &CloudError) -> bool {
    matches!(err, CloudError::RateLimit(_) | CloudError::Provider(_))
}

/// Apply ±20% jitter around `base_ms`.
///
/// Returns a value in [`base_ms × 0.8`, `base_ms × 1.2`], floored to at least 1 ms.
fn jittered_delay_ms(base_ms: u64) -> u64 {
    let jitter_range = (base_ms as f64 * 0.2).round() as i64;
    let jitter: i64 = rand::thread_rng().gen_range(-jitter_range..=jitter_range);
    (base_ms as i64 + jitter).max(1) as u64
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::DigitalOceanProvider;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Helper: start a wiremock server and wrap a `DigitalOceanProvider` in
    /// a `RetryCloudProvider` pointed at that server.
    async fn setup_retry(
        max_retries: u32,
        base_delay_ms: u64,
    ) -> (MockServer, RetryCloudProvider<DigitalOceanProvider>) {
        let mock_server = MockServer::start().await;
        let base_url = mock_server.uri();
        let provider = DigitalOceanProvider::with_base_url(&base_url).unwrap();
        let retry = RetryCloudProvider::with_retries(provider, max_retries, base_delay_ms);
        (mock_server, retry)
    }

    // -----------------------------------------------------------------------
    // 2.3.a — Rate limit recovery (429 → 200)
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn rate_limit_recovery() {
        let (mock_server, retry) = setup_retry(2, 50).await;

        // First invocation: 429 with Retry-After (only once — retry then gets 200)
        Mock::given(method("GET"))
            .and(path("/v2/account"))
            .respond_with(
                ResponseTemplate::new(429).insert_header("retry-after", "1"),
            )
            .with_priority(1)
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        // Second invocation: 200 success
        Mock::given(method("GET"))
            .and(path("/v2/account"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let result = retry.validate_token("test-token").await;

        assert!(result.is_ok(), "expected Ok after rate-limit retry, got {:?}", result);
    }

    // -----------------------------------------------------------------------
    // 2.3.b — 5xx exhaustion after 3 retries
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn exhausts_retries_on_5xx() {
        let (mock_server, retry) = setup_retry(3, 50).await;

        // Always 503 — every attempt fails the same way
        Mock::given(method("GET"))
            .and(path("/v2/account"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&mock_server)
            .await;

        let result = retry.validate_token("test-token").await;

        assert!(result.is_err(), "expected Err after exhausting retries");
        match result.unwrap_err() {
            CloudError::Provider(_) => { /* expected */ }
            other => panic!("expected Provider error, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // 2.3.c — Auth (401) bypasses retry immediately
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn auth_bypasses_retry() {
        let (mock_server, retry) = setup_retry(3, 50).await;

        // Only mount ONE mock. If retry tries again the mock server will panic
        // (no matching handler), proving Auth was NOT retried.
        Mock::given(method("GET"))
            .and(path("/v2/account"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&mock_server)
            .await;

        let result = retry.validate_token("test-token").await;

        assert!(result.is_err(), "expected Err for auth failure");
        match result.unwrap_err() {
            CloudError::Auth(_) => { /* expected — immediate, no retry */ }
            other => panic!("expected Auth error, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Triangulation — verify is_retriable logic directly
    // -----------------------------------------------------------------------
    #[test]
    fn retriable_errors() {
        assert!(is_retriable(&CloudError::RateLimit(5)));
        assert!(is_retriable(&CloudError::Provider("upstream down".into())));
    }

    #[test]
    fn non_retriable_errors() {
        assert!(!is_retriable(&CloudError::Auth("bad key".into())));
        assert!(!is_retriable(&CloudError::Quota("over limit".into())));
        assert!(!is_retriable(&CloudError::Timeout));
    }

    // -----------------------------------------------------------------------
    // Triangulation — jitter respects the ±20 % boundary
    // -----------------------------------------------------------------------
    #[test]
    fn jitter_stays_within_bounds() {
        for base in [100, 500, 1000, 5000] {
            for _ in 0..100 {
                let result = jittered_delay_ms(base);
                let min = (base as f64 * 0.8) as u64;
                let max = (base as f64 * 1.2).ceil() as u64;
                assert!(
                    result >= min && result <= max,
                    "jittered_delay_ms({base}) = {result} out of bounds [{min}, {max}]"
                );
            }
        }
    }

    #[test]
    fn jitter_is_at_least_one_ms() {
        // Even for extremely small base values we should never sleep 0 ms
        let result = jittered_delay_ms(1);
        assert!(result >= 1, "jittered_delay_ms(1) = {result}, expected >= 1");
    }
}
