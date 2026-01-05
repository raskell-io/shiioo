use crate::config::{AppState, ServerConfig};
use crate::ui;
use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
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
        // Role management
        .route("/api/roles", get(handlers::list_roles))
        .route("/api/roles", post(handlers::create_role))
        .route("/api/roles/{role_id}", get(handlers::get_role))
        .route("/api/roles/{role_id}", delete(handlers::delete_role))
        // Policy management
        .route("/api/policies", get(handlers::list_policies))
        .route("/api/policies", post(handlers::create_policy))
        .route("/api/policies/{policy_id}", get(handlers::get_policy))
        .route("/api/policies/{policy_id}", delete(handlers::delete_policy))
        // Organization management
        .route("/api/organizations", get(handlers::list_organizations))
        .route("/api/organizations", post(handlers::create_organization))
        .route("/api/organizations/{org_id}", get(handlers::get_organization))
        .route("/api/organizations/{org_id}", delete(handlers::delete_organization))
        // Template management
        .route("/api/templates", get(handlers::list_templates))
        .route("/api/templates", post(handlers::create_template))
        .route("/api/templates/{template_id}", get(handlers::get_template))
        .route("/api/templates/{template_id}", delete(handlers::delete_template))
        .route("/api/templates/{template_id}/instantiate", post(handlers::instantiate_template))
        // Claude config compiler
        .route("/api/claude/compile/{role_id}", get(handlers::compile_claude_config))
        // Capacity management
        .route("/api/capacity/sources", get(handlers::list_capacity_sources))
        .route("/api/capacity/sources", post(handlers::create_capacity_source))
        .route("/api/capacity/sources/{source_id}", get(handlers::get_capacity_source))
        .route("/api/capacity/sources/{source_id}", delete(handlers::delete_capacity_source))
        .route("/api/capacity/usage", get(handlers::list_capacity_usage))
        .route("/api/capacity/cost", get(handlers::get_capacity_cost))
        // Routine management (Phase 5)
        .route("/api/routines", get(handlers::list_routines))
        .route("/api/routines", post(handlers::create_routine))
        .route("/api/routines/{routine_id}", get(handlers::get_routine))
        .route("/api/routines/{routine_id}", delete(handlers::delete_routine))
        .route("/api/routines/{routine_id}/enable", post(handlers::enable_routine))
        .route("/api/routines/{routine_id}/disable", post(handlers::disable_routine))
        .route("/api/routines/{routine_id}/executions", get(handlers::get_routine_executions))
        // Approval board management (Phase 5)
        .route("/api/approval-boards", get(handlers::list_approval_boards))
        .route("/api/approval-boards", post(handlers::create_approval_board))
        .route("/api/approval-boards/{board_id}", get(handlers::get_approval_board))
        .route("/api/approval-boards/{board_id}", delete(handlers::delete_approval_board))
        // Approval management (Phase 5)
        .route("/api/approvals", get(handlers::list_approvals))
        .route("/api/approvals/{approval_id}", get(handlers::get_approval))
        .route("/api/approvals/{approval_id}/vote", post(handlers::cast_vote))
        // Config change management (Phase 5)
        .route("/api/config-changes", get(handlers::list_config_changes))
        .route("/api/config-changes", post(handlers::propose_config_change))
        .route("/api/config-changes/{change_id}", get(handlers::get_config_change))
        .route("/api/config-changes/{change_id}/apply", post(handlers::apply_config_change))
        .route("/api/config-changes/{change_id}/reject", post(handlers::reject_config_change))
        // Observability (Phase 6)
        .route("/api/metrics", get(handlers::get_metrics))
        .route("/api/analytics/workflows", get(handlers::get_workflow_analytics))
        .route("/api/analytics/workflows/{workflow_id}", get(handlers::get_workflow_analytics_by_id))
        .route("/api/analytics/steps", get(handlers::get_step_analytics))
        .route("/api/analytics/traces", get(handlers::get_execution_traces))
        .route("/api/analytics/traces/{run_id}", get(handlers::get_execution_trace))
        .route("/api/analytics/bottlenecks/{workflow_id}", get(handlers::get_bottleneck_analysis))
        .route("/api/health/status", get(handlers::get_health_status))
        // WebSocket for real-time updates
        .route("/api/ws", get(crate::websocket::ws_handler))
        // Multi-tenancy (Phase 7)
        .route("/api/tenants", get(handlers::list_tenants))
        .route("/api/tenants", post(handlers::register_tenant))
        .route("/api/tenants/{tenant_id}", get(handlers::get_tenant))
        .route("/api/tenants/{tenant_id}", axum::routing::put(handlers::update_tenant))
        .route("/api/tenants/{tenant_id}", delete(handlers::delete_tenant))
        .route("/api/tenants/{tenant_id}/suspend", post(handlers::suspend_tenant))
        .route("/api/tenants/{tenant_id}/activate", post(handlers::activate_tenant))
        .route("/api/tenants/{tenant_id}/storage-stats", get(handlers::get_tenant_storage_stats))
        // Cluster management (Phase 7)
        .route("/api/cluster/nodes", get(handlers::list_cluster_nodes))
        .route("/api/cluster/nodes", post(handlers::register_cluster_node))
        .route("/api/cluster/nodes/{node_id}", get(handlers::get_cluster_node))
        .route("/api/cluster/nodes/{node_id}", delete(handlers::remove_cluster_node))
        .route("/api/cluster/nodes/{node_id}/heartbeat", post(handlers::node_heartbeat))
        .route("/api/cluster/leader", get(handlers::get_cluster_leader))
        .route("/api/cluster/health", get(handlers::get_cluster_health))
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
