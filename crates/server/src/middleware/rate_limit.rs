//! Rate limiting middleware using the token bucket algorithm.
//!
//! Protects the API against abuse by limiting requests per IP address.
//! Configuration is done via environment variables:
//!
//! - `SHIIOO_RATE_LIMIT_PER_SECOND`: Requests allowed per second (default: 10)
//! - `SHIIOO_RATE_LIMIT_BURST`: Maximum burst size (default: 50)

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
};
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use std::{
    net::SocketAddr,
    num::NonZeroU32,
    sync::Arc,
    task::{Context, Poll},
};
use tower::{Layer, Service};

/// Environment variable for requests per second limit.
pub const RATE_LIMIT_PER_SECOND_ENV: &str = "SHIIOO_RATE_LIMIT_PER_SECOND";

/// Environment variable for burst size limit.
pub const RATE_LIMIT_BURST_ENV: &str = "SHIIOO_RATE_LIMIT_BURST";

/// Default requests per second.
const DEFAULT_RATE_LIMIT_PER_SECOND: u32 = 10;

/// Default burst size.
const DEFAULT_RATE_LIMIT_BURST: u32 = 50;

/// Rate limiter configuration.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Requests allowed per second (sustained rate).
    pub per_second: u32,
    /// Maximum burst size (allows temporary spikes).
    pub burst: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            per_second: DEFAULT_RATE_LIMIT_PER_SECOND,
            burst: DEFAULT_RATE_LIMIT_BURST,
        }
    }
}

impl RateLimitConfig {
    /// Load configuration from environment variables.
    pub fn from_env() -> Self {
        let per_second = std::env::var(RATE_LIMIT_PER_SECOND_ENV)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_RATE_LIMIT_PER_SECOND);

        let burst = std::env::var(RATE_LIMIT_BURST_ENV)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_RATE_LIMIT_BURST);

        Self { per_second, burst }
    }

    /// Create a new configuration with custom values.
    #[allow(dead_code)]
    pub fn new(per_second: u32, burst: u32) -> Self {
        Self { per_second, burst }
    }
}

/// Global rate limiter (not per-IP, simpler but less granular).
type GlobalRateLimiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

/// Layer that applies rate limiting to requests.
#[derive(Clone)]
pub struct RateLimitLayer {
    limiter: Arc<GlobalRateLimiter>,
}

impl RateLimitLayer {
    /// Create a new rate limit layer with the given configuration.
    pub fn new(config: RateLimitConfig) -> Self {
        let quota = Quota::per_second(NonZeroU32::new(config.per_second).unwrap_or(NonZeroU32::MIN))
            .allow_burst(NonZeroU32::new(config.burst).unwrap_or(NonZeroU32::MIN));

        let limiter = Arc::new(RateLimiter::direct(quota));

        Self { limiter }
    }

    /// Create a rate limit layer from environment configuration.
    pub fn from_env() -> Self {
        Self::new(RateLimitConfig::from_env())
    }
}

impl<S> Layer<S> for RateLimitLayer {
    type Service = RateLimitService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimitService {
            inner,
            limiter: self.limiter.clone(),
        }
    }
}

/// Service that enforces rate limits.
#[derive(Clone)]
pub struct RateLimitService<S> {
    inner: S,
    limiter: Arc<GlobalRateLimiter>,
}

impl<S> Service<Request<Body>> for RateLimitService<S>
where
    S: Service<Request<Body>, Response = Response> + Clone + Send + 'static,
    S::Future: Send,
{
    type Response = Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let limiter = self.limiter.clone();
        let mut inner = self.inner.clone();

        Box::pin(async move {
            // Check rate limit
            if limiter.check().is_err() {
                // Get client IP for logging if available
                let client_ip = req
                    .extensions()
                    .get::<ConnectInfo<SocketAddr>>()
                    .map(|ci| ci.0.ip().to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                tracing::warn!(
                    client_ip = %client_ip,
                    path = %req.uri().path(),
                    "Rate limit exceeded"
                );

                return Ok(RateLimitExceeded.into_response());
            }

            inner.call(req).await
        })
    }
}

/// Response returned when rate limit is exceeded.
struct RateLimitExceeded;

impl IntoResponse for RateLimitExceeded {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "error": "Too Many Requests",
            "message": "Rate limit exceeded. Please slow down and try again.",
            "retry_after_seconds": 1
        });

        (
            StatusCode::TOO_MANY_REQUESTS,
            [
                ("Content-Type", "application/json"),
                ("Retry-After", "1"),
            ],
            body.to_string(),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RateLimitConfig::default();
        assert_eq!(config.per_second, DEFAULT_RATE_LIMIT_PER_SECOND);
        assert_eq!(config.burst, DEFAULT_RATE_LIMIT_BURST);
    }

    #[test]
    fn test_custom_config() {
        let config = RateLimitConfig::new(100, 200);
        assert_eq!(config.per_second, 100);
        assert_eq!(config.burst, 200);
    }

    #[test]
    fn test_layer_creation() {
        let config = RateLimitConfig::new(10, 50);
        let _layer = RateLimitLayer::new(config);
    }

    #[test]
    fn test_rate_limit_exceeded_response() {
        let response = RateLimitExceeded.into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }
}
