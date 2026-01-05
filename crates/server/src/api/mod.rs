use crate::config::{AppState, ServerConfig};
use crate::ui;
use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_http::{
    cors::CorsLayer,
    trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer},
};

mod handlers;

/// Start the API server
pub async fn serve(addr: &str, config: ServerConfig) -> Result<()> {
    let state = AppState::new(&config)?;

    let app = create_router(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("API server listening on {}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

/// Create the API router
fn create_router(state: AppState) -> Router {
    Router::new()
        // API routes
        .route("/api/health", get(health_check))
        .route("/api/runs", get(handlers::list_runs))
        .route("/api/runs/{run_id}", get(handlers::get_run))
        .route("/api/runs/{run_id}/events", get(handlers::get_run_events))
        .route("/api/jobs", post(handlers::create_job))
        // UI routes
        .fallback(ui::serve_ui)
        // Middleware
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().include_headers(true))
                .on_response(DefaultOnResponse::new().include_headers(true)),
        )
        .layer(CorsLayer::permissive())
        .with_state(Arc::new(state))
}

/// Health check endpoint
async fn health_check() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "service": "shiioo",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// API error response
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

impl ErrorResponse {
    pub fn new(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            details: None,
        }
    }

    pub fn with_details(error: impl Into<String>, details: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            details: Some(details.into()),
        }
    }
}

/// Custom error type for API handlers
pub struct ApiError(anyhow::Error);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let error_msg = self.0.to_string();
        let details = self.0.chain().skip(1).map(|e| e.to_string()).collect::<Vec<_>>().join(": ");

        let response = if details.is_empty() {
            ErrorResponse::new(error_msg)
        } else {
            ErrorResponse::with_details(error_msg, details)
        };

        (StatusCode::INTERNAL_SERVER_ERROR, Json(response)).into_response()
    }
}

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
