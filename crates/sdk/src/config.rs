//! Configuration types for the Shiioo SDK.

use std::time::Duration;
use url::Url;

/// Configuration for the Shiioo client.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Base URL of the Shiioo server.
    pub base_url: Url,
    /// API key for authentication.
    pub api_key: Option<String>,
    /// Request timeout.
    pub timeout: Duration,
    /// Retry configuration.
    pub retry_config: RetryConfig,
    /// Tenant ID for multi-tenant operations.
    pub tenant_id: Option<String>,
}

impl ClientConfig {
    /// Create a new configuration with the given base URL.
    pub fn new(base_url: Url) -> Self {
        Self {
            base_url,
            api_key: None,
            timeout: Duration::from_secs(30),
            retry_config: RetryConfig::default(),
            tenant_id: None,
        }
    }
}

/// Configuration for retry behavior.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retries.
    pub max_retries: u32,
    /// Initial backoff duration.
    pub initial_backoff: Duration,
    /// Maximum backoff duration.
    pub max_backoff: Duration,
    /// Backoff multiplier.
    pub backoff_multiplier: f64,
    /// HTTP status codes to retry on.
    pub retry_on_status_codes: Vec<u16>,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(10),
            backoff_multiplier: 2.0,
            retry_on_status_codes: vec![429, 500, 502, 503, 504],
        }
    }
}

impl RetryConfig {
    /// Create a configuration with no retries.
    pub fn no_retry() -> Self {
        Self {
            max_retries: 0,
            ..Default::default()
        }
    }

    /// Calculate backoff duration for a given attempt.
    pub fn backoff_for_attempt(&self, attempt: u32) -> Duration {
        let backoff_ms = self.initial_backoff.as_millis() as f64
            * self.backoff_multiplier.powi(attempt as i32);
        let backoff = Duration::from_millis(backoff_ms as u64);
        std::cmp::min(backoff, self.max_backoff)
    }

    /// Check if a status code should trigger a retry.
    pub fn should_retry_status(&self, status: u16) -> bool {
        self.retry_on_status_codes.contains(&status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_calculation() {
        let config = RetryConfig::default();

        // First attempt: 100ms
        assert_eq!(config.backoff_for_attempt(0), Duration::from_millis(100));
        // Second attempt: 200ms
        assert_eq!(config.backoff_for_attempt(1), Duration::from_millis(200));
        // Third attempt: 400ms
        assert_eq!(config.backoff_for_attempt(2), Duration::from_millis(400));
    }

    #[test]
    fn test_backoff_capped_at_max() {
        let config = RetryConfig {
            max_backoff: Duration::from_millis(500),
            ..Default::default()
        };

        // Should be capped at max
        assert_eq!(config.backoff_for_attempt(10), Duration::from_millis(500));
    }

    #[test]
    fn test_should_retry_status() {
        let config = RetryConfig::default();

        assert!(config.should_retry_status(429));
        assert!(config.should_retry_status(500));
        assert!(config.should_retry_status(503));
        assert!(!config.should_retry_status(400));
        assert!(!config.should_retry_status(404));
    }

    #[test]
    fn test_client_config_new() {
        let url = Url::parse("https://example.com").unwrap();
        let config = ClientConfig::new(url.clone());

        assert_eq!(config.base_url, url);
        assert!(config.api_key.is_none());
        assert!(config.tenant_id.is_none());
    }

    #[test]
    fn test_client_config_defaults() {
        let url = Url::parse("https://example.com").unwrap();
        let config = ClientConfig::new(url);

        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.retry_config.max_retries, 3);
    }

    #[test]
    fn test_retry_config_no_retry() {
        let config = RetryConfig::no_retry();

        assert_eq!(config.max_retries, 0);
        // Other defaults should still be present
        assert_eq!(config.initial_backoff, Duration::from_millis(100));
    }

    #[test]
    fn test_retry_config_defaults() {
        let config = RetryConfig::default();

        assert_eq!(config.max_retries, 3);
        assert_eq!(config.initial_backoff, Duration::from_millis(100));
        assert_eq!(config.max_backoff, Duration::from_secs(10));
        assert_eq!(config.backoff_multiplier, 2.0);
        assert_eq!(config.retry_on_status_codes, vec![429, 500, 502, 503, 504]);
    }
}
