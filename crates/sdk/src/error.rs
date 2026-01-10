//! Error types for the Shiioo SDK.

use serde::{Deserialize, Serialize};

/// Result type for SDK operations.
pub type ShiiooResult<T> = Result<T, ShiiooError>;

/// Error types that can occur when using the Shiioo SDK.
#[derive(Debug, thiserror::Error)]
pub enum ShiiooError {
    /// HTTP request failed.
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// API returned an error response.
    #[error("API error (status {status}): {message}")]
    Api {
        status: u16,
        message: String,
        details: Option<String>,
    },

    /// Invalid configuration.
    #[error("Configuration error: {0}")]
    Config(String),

    /// Serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// WebSocket error.
    #[error("WebSocket error: {0}")]
    WebSocket(String),

    /// Connection timeout.
    #[error("Request timed out")]
    Timeout,

    /// Authentication failed.
    #[error("Authentication failed: {0}")]
    Authentication(String),

    /// Resource not found.
    #[error("Resource not found: {0}")]
    NotFound(String),

    /// Rate limited.
    #[error("Rate limited, retry after {retry_after_secs:?} seconds")]
    RateLimited { retry_after_secs: Option<u64> },

    /// Invalid input.
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    /// URL parsing error.
    #[error("Invalid URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
}

impl ShiiooError {
    /// Check if this error is retryable.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Http(_) | Self::Timeout | Self::RateLimited { .. } => true,
            Self::Api { status, .. } => *status >= 500,
            _ => false,
        }
    }

    /// Create an API error from a status code and response body.
    pub fn from_response(status: u16, body: &str) -> Self {
        // Try to parse as ErrorResponse
        if let Ok(error_response) = serde_json::from_str::<ErrorResponse>(body) {
            Self::Api {
                status,
                message: error_response.error,
                details: error_response.details,
            }
        } else {
            Self::Api {
                status,
                message: body.to_string(),
                details: None,
            }
        }
    }
}

/// Error response from the Shiioo API.
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_retryable_timeout() {
        assert!(ShiiooError::Timeout.is_retryable());
    }

    #[test]
    fn test_is_retryable_rate_limited() {
        let error = ShiiooError::RateLimited {
            retry_after_secs: Some(30),
        };
        assert!(error.is_retryable());

        let error_no_retry = ShiiooError::RateLimited {
            retry_after_secs: None,
        };
        assert!(error_no_retry.is_retryable());
    }

    #[test]
    fn test_is_retryable_api_5xx() {
        let error_500 = ShiiooError::Api {
            status: 500,
            message: "Internal Server Error".to_string(),
            details: None,
        };
        assert!(error_500.is_retryable());

        let error_503 = ShiiooError::Api {
            status: 503,
            message: "Service Unavailable".to_string(),
            details: None,
        };
        assert!(error_503.is_retryable());
    }

    #[test]
    fn test_is_retryable_api_4xx_not_retryable() {
        let error_400 = ShiiooError::Api {
            status: 400,
            message: "Bad Request".to_string(),
            details: None,
        };
        assert!(!error_400.is_retryable());

        let error_404 = ShiiooError::Api {
            status: 404,
            message: "Not Found".to_string(),
            details: None,
        };
        assert!(!error_404.is_retryable());
    }

    #[test]
    fn test_is_retryable_auth_error() {
        let error = ShiiooError::Authentication("Invalid token".to_string());
        assert!(!error.is_retryable());
    }

    #[test]
    fn test_is_retryable_not_found() {
        let error = ShiiooError::NotFound("Run not found".to_string());
        assert!(!error.is_retryable());
    }

    #[test]
    fn test_is_retryable_config() {
        let error = ShiiooError::Config("Missing base URL".to_string());
        assert!(!error.is_retryable());
    }

    #[test]
    fn test_from_response_json() {
        let body = r#"{"error": "Something went wrong", "details": "More info here"}"#;
        let error = ShiiooError::from_response(500, body);

        match error {
            ShiiooError::Api {
                status,
                message,
                details,
            } => {
                assert_eq!(status, 500);
                assert_eq!(message, "Something went wrong");
                assert_eq!(details, Some("More info here".to_string()));
            }
            _ => panic!("Expected Api error"),
        }
    }

    #[test]
    fn test_from_response_plain_text() {
        let body = "Plain text error message";
        let error = ShiiooError::from_response(400, body);

        match error {
            ShiiooError::Api {
                status,
                message,
                details,
            } => {
                assert_eq!(status, 400);
                assert_eq!(message, "Plain text error message");
                assert!(details.is_none());
            }
            _ => panic!("Expected Api error"),
        }
    }

    #[test]
    fn test_error_display() {
        let timeout = ShiiooError::Timeout;
        assert_eq!(format!("{}", timeout), "Request timed out");

        let api = ShiiooError::Api {
            status: 404,
            message: "Not found".to_string(),
            details: None,
        };
        assert_eq!(format!("{}", api), "API error (status 404): Not found");

        let config = ShiiooError::Config("Missing URL".to_string());
        assert_eq!(format!("{}", config), "Configuration error: Missing URL");
    }
}
