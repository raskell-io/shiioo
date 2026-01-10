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
