use crate::config::{AppState, ServerConfig};
use crate::middleware::RateLimitLayer;
use crate::ui;
use anyhow::Result;
use axum::{
    extract::State,
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_http::{
    cors::CorsLayer,
    trace::{DefaultOnResponse, TraceLayer},
};

mod agent_handlers;
mod handlers;
mod migration_handlers;
mod runtime_handlers;

/// Start the API server with graceful shutdown
pub async fn serve(addr: &str, config: ServerConfig) -> Result<()> {
    let state = AppState::new(&config)?;

    // Build GraphQL schema (Phase 10)
    let schema = crate::graphql::build_schema(Arc::new(state.clone()));

    let app = create_router(state, schema);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(addr = %addr, "API server listening");

    // Graceful shutdown handling
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Wait for shutdown signal (SIGTERM or SIGINT)
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("Received Ctrl+C, starting graceful shutdown");
        }
        _ = terminate => {
            tracing::info!("Received SIGTERM, starting graceful shutdown");
        }
    }

    // Give in-flight requests time to complete
    tracing::info!("Waiting for in-flight requests to complete...");
}

/// Middleware to add request ID to each request
///
/// If the client provides an `X-Request-ID` header, it's used.
/// Otherwise, a new UUID is generated.
/// The request ID is added to the response headers for correlation.
async fn request_id_middleware(
    mut request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    // Get or generate request ID
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // Insert request ID into request headers if not present
    if !request.headers().contains_key("x-request-id") {
        request.headers_mut().insert(
            "x-request-id",
            request_id.parse().unwrap(),
        );
    }

    // Process request
    let mut response = next.run(request).await;

    // Add request ID to response headers
    response.headers_mut().insert(
        "x-request-id",
        request_id.parse().unwrap(),
    );

    response
}

/// Create the API router
fn create_router(state: AppState, schema: crate::graphql::ShiiooSchema) -> Router {
    Router::new()
        // GraphQL endpoints (Phase 10)
        .route("/api/graphql", post(crate::graphql::graphql_handler))
        .route("/api/graphql", get(crate::graphql::graphql_playground))
        .route("/api/graphql/ws", get(crate::graphql::graphql_subscription_handler))
        // Health probes (Kubernetes)
        .route("/health/live", get(liveness_probe))
        .route("/health/ready", get(readiness_probe))
        // Prometheus metrics endpoint
        .route("/metrics", get(handlers::prometheus_metrics))
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
        // Secret Management (Phase 8)
        .route("/api/secrets", get(handlers::list_secrets))
        .route("/api/secrets", post(handlers::create_secret))
        .route("/api/secrets/{secret_id}", get(handlers::get_secret))
        .route("/api/secrets/{secret_id}", delete(handlers::delete_secret))
        .route("/api/secrets/{secret_id}", axum::routing::put(handlers::update_secret_metadata))
        .route("/api/secrets/{secret_id}/value", get(handlers::get_secret_value))
        .route("/api/secrets/{secret_id}/rotate", post(handlers::rotate_secret))
        .route("/api/secrets/{secret_id}/versions", get(handlers::get_secret_versions))
        .route("/api/secrets/rotation/needed", get(handlers::get_secrets_needing_rotation))
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
        // Audit logging (Phase 9)
        .route("/api/audit/entries", get(handlers::list_audit_entries))
        .route("/api/audit/statistics", get(handlers::get_audit_statistics))
        .route("/api/audit/verify-chain", get(handlers::verify_audit_chain))
        // RBAC (Phase 9)
        .route("/api/rbac/roles", get(handlers::list_rbac_roles))
        .route("/api/rbac/roles", post(handlers::create_rbac_role))
        .route("/api/rbac/roles/{role_id}", get(handlers::get_rbac_role))
        .route("/api/rbac/assign-role", post(handlers::assign_user_role))
        .route("/api/rbac/check-permission", post(handlers::check_user_permission))
        // Compliance & Security (Phase 9)
        .route("/api/compliance/report", post(handlers::generate_compliance_report))
        .route("/api/security/scan", post(handlers::run_security_scan))
        // Agent management (Phase 12 - Agent Primitive)
        .route("/api/agents", get(agent_handlers::list_agents))
        .route("/api/agents", post(agent_handlers::create_agent))
        .route("/api/agents/{agent_id}", get(agent_handlers::get_agent))
        .route("/api/agents/{agent_id}", axum::routing::put(agent_handlers::update_agent))
        .route("/api/agents/{agent_id}", delete(agent_handlers::delete_agent))
        .route("/api/agents/{agent_id}/activate", post(agent_handlers::activate_agent))
        .route("/api/agents/{agent_id}/suspend", post(agent_handlers::suspend_agent))
        .route("/api/agents/{agent_id}/pause", post(agent_handlers::pause_agent))
        .route("/api/agents/{agent_id}/archive", post(agent_handlers::archive_agent))
        // Archetype management (Phase 12 - Agent Primitive)
        .route("/api/archetypes", get(agent_handlers::list_archetypes))
        .route("/api/archetypes", post(agent_handlers::create_archetype))
        .route("/api/archetypes/{archetype_id}", get(agent_handlers::get_archetype))
        .route("/api/archetypes/{archetype_id}", axum::routing::put(agent_handlers::update_archetype))
        .route("/api/archetypes/{archetype_id}", delete(agent_handlers::delete_archetype))
        // Migration (Phase 12 - Agent Primitive)
        .route("/api/migration/run", post(migration_handlers::run_migration))
        .route("/api/migration/preview", post(migration_handlers::preview_migration))
        .route("/api/migration/roles", post(migration_handlers::migrate_roles))
        .route("/api/migration/persons", post(migration_handlers::migrate_persons))
        // Agent Runtime (Phase 13-14)
        .route("/api/agents/{agent_id}/execute", post(runtime_handlers::execute_task))
        .route("/api/agents/{agent_id}/budget", get(runtime_handlers::get_agent_budget))
        .route("/api/agents/{agent_id}/delegate", post(runtime_handlers::delegate_task))
        .route("/api/agents/{agent_id}/supervisor", post(runtime_handlers::create_supervisor))
        .route("/api/agents/{agent_id}/supervision-events", get(runtime_handlers::get_supervision_events))
        // Agent Teams (Phase 13-14)
        .route("/api/teams", get(runtime_handlers::list_teams))
        .route("/api/teams", post(runtime_handlers::create_team))
        .route("/api/teams/{team_id}", get(runtime_handlers::get_team))
        .route("/api/teams/{team_id}/broadcast", post(runtime_handlers::broadcast_to_team))
        // Workflow-Agent Integration (Phase 13-14)
        .route("/api/workflow/execute-step", post(runtime_handlers::execute_workflow_step))
        // UI routes (Phase 10)
        .route("/dashboard", get(ui::serve_dashboard))
        .fallback(ui::serve_ui)
        // Middleware (applied in reverse order - last added is first executed)
        .layer(middleware::from_fn(request_id_middleware))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request<axum::body::Body>| {
                    let request_id = request
                        .headers()
                        .get("x-request-id")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("-");
                    tracing::info_span!(
                        "http_request",
                        request_id = %request_id,
                        method = %request.method(),
                        uri = %request.uri(),
                    )
                })
                .on_response(DefaultOnResponse::new().include_headers(true)),
        )
        .layer(CorsLayer::permissive())
        .layer(RateLimitLayer::from_env())
        .layer(axum::Extension(schema))
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

/// Kubernetes liveness probe
///
/// Returns 200 OK if the process is alive and can handle requests.
/// If this fails, Kubernetes will restart the container.
async fn liveness_probe() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({
        "status": "alive",
    })))
}

/// Kubernetes readiness probe
///
/// Returns 200 OK if the service is ready to accept traffic.
/// Checks that critical dependencies (storage, etc.) are accessible.
/// If this fails, Kubernetes won't route traffic to this pod.
async fn readiness_probe(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let mut checks = Vec::new();
    let mut all_healthy = true;

    // Check index store (redb database)
    let index_ok = state.index_store.list_runs().is_ok();
    checks.push(serde_json::json!({
        "name": "index_store",
        "status": if index_ok { "ok" } else { "error" },
    }));
    if !index_ok {
        all_healthy = false;
    }

    // Check cluster manager
    let cluster_ok = !state.cluster_manager.list_healthy_nodes().is_empty()
        || state.cluster_manager.list_nodes().is_empty(); // OK if no cluster configured
    checks.push(serde_json::json!({
        "name": "cluster",
        "status": if cluster_ok { "ok" } else { "degraded" },
    }));

    let status = if all_healthy { "ready" } else { "not_ready" };
    let status_code = if all_healthy { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };

    (status_code, Json(serde_json::json!({
        "status": status,
        "checks": checks,
    })))
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
