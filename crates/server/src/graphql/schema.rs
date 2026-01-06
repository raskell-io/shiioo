use async_graphql::*;
use chrono::{DateTime, Utc};
use futures::Stream;
use shiioo_core::*;
use std::sync::Arc;

use crate::config::AppState;

/// GraphQL workflow type
#[derive(Clone, SimpleObject)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub steps: Vec<WorkflowStep>,
}

/// GraphQL workflow step
#[derive(Clone, SimpleObject)]
pub struct WorkflowStep {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub role: String,
    pub action_type: String,
}

/// GraphQL run status
#[derive(Clone, SimpleObject)]
pub struct Run {
    pub id: String,
    pub workflow_id: String,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

/// GraphQL audit entry
#[derive(Clone, SimpleObject)]
pub struct AuditEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub category: String,
    pub severity: String,
    pub user_id: Option<String>,
    pub tenant_id: Option<String>,
}

/// GraphQL tenant
#[derive(Clone, SimpleObject)]
pub struct Tenant {
    pub id: String,
    pub name: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

/// GraphQL cluster node
#[derive(Clone, SimpleObject)]
pub struct ClusterNode {
    pub id: String,
    pub address: String,
    pub status: String,
    pub last_heartbeat: DateTime<Utc>,
}

/// GraphQL metrics summary
#[derive(Clone, SimpleObject)]
pub struct MetricsSummary {
    pub total_runs: i64,
    pub successful_runs: i64,
    pub failed_runs: i64,
    pub avg_duration_ms: f64,
}

/// Query root
pub struct Query;

#[Object]
impl Query {
    /// Get workflow by ID
    async fn workflow(&self, ctx: &Context<'_>, id: String) -> Result<Option<Workflow>> {
        let state = ctx.data::<Arc<AppState>>()?;

        // For now, return a placeholder
        // In a real implementation, we'd query from storage
        Ok(Some(Workflow {
            id: id.clone(),
            name: "Sample Workflow".to_string(),
            description: Some("A sample workflow".to_string()),
            created_at: Utc::now(),
            steps: vec![],
        }))
    }

    /// List all workflows
    async fn workflows(&self, ctx: &Context<'_>, limit: Option<i32>) -> Result<Vec<Workflow>> {
        let state = ctx.data::<Arc<AppState>>()?;
        let limit = limit.unwrap_or(100);

        // Placeholder implementation
        Ok(vec![])
    }

    /// Get run by ID
    async fn run(&self, ctx: &Context<'_>, id: String) -> Result<Option<Run>> {
        let state = ctx.data::<Arc<AppState>>()?;

        // Try to get run from index
        let run_opt = match state.index_store.get_run(&RunId(uuid::Uuid::parse_str(&id)?)) {
            Ok(opt) => opt,
            Err(_) => return Ok(None),
        };

        match run_opt {
            Some(run_data) => Ok(Some(Run {
                id: run_data.id.0.to_string(),
                workflow_id: run_data.work_item_id.clone(),
                status: format!("{:?}", run_data.status),
                started_at: run_data.started_at,
                completed_at: run_data.completed_at,
                error: None, // Run struct doesn't have error field
            })),
            None => Ok(None),
        }
    }

    /// List recent runs
    async fn runs(&self, ctx: &Context<'_>, limit: Option<i32>) -> Result<Vec<Run>> {
        let state = ctx.data::<Arc<AppState>>()?;
        let limit = limit.unwrap_or(100) as usize;

        let runs = state.index_store.list_runs()?;

        Ok(runs
            .into_iter()
            .take(limit)
            .map(|r| Run {
                id: r.id.0.to_string(),
                workflow_id: r.work_item_id.clone(),
                status: format!("{:?}", r.status),
                started_at: r.started_at,
                completed_at: r.completed_at,
                error: None, // Run struct doesn't have error field
            })
            .collect())
    }

    /// Get audit log entries
    async fn audit_entries(
        &self,
        ctx: &Context<'_>,
        limit: Option<i32>,
        category: Option<String>,
    ) -> Result<Vec<AuditEntry>> {
        let state = ctx.data::<Arc<AppState>>()?;
        let limit = limit.unwrap_or(100) as usize;

        let entries = if let Some(cat) = category {
            // Filter by category
            let category = match cat.as_str() {
                "Authentication" => audit::AuditCategory::Authentication,
                "Authorization" => audit::AuditCategory::Authorization,
                "DataAccess" => audit::AuditCategory::DataAccess,
                "DataModification" => audit::AuditCategory::DataModification,
                "ConfigChange" => audit::AuditCategory::ConfigChange,
                "SecretAccess" => audit::AuditCategory::SecretAccess,
                "WorkflowExecution" => audit::AuditCategory::WorkflowExecution,
                "SystemEvent" => audit::AuditCategory::SystemEvent,
                "SecurityEvent" => audit::AuditCategory::SecurityEvent,
                "ComplianceEvent" => audit::AuditCategory::ComplianceEvent,
                _ => return Err(Error::new("Invalid category")),
            };
            state.audit_log.filter_by_category(category)
        } else {
            state.audit_log.list_all()
        };

        Ok(entries
            .into_iter()
            .take(limit)
            .map(|e| AuditEntry {
                id: e.id.0.clone(),
                timestamp: e.timestamp,
                category: format!("{:?}", e.category),
                severity: format!("{:?}", e.severity),
                user_id: e.user_id,
                tenant_id: e.tenant_id,
            })
            .collect())
    }

    /// Get tenants
    async fn tenants(&self, ctx: &Context<'_>) -> Result<Vec<Tenant>> {
        let state = ctx.data::<Arc<AppState>>()?;

        let tenants = state.tenant_manager.list_tenants();

        Ok(tenants
            .into_iter()
            .map(|t| Tenant {
                id: t.id.0.clone(),
                name: t.name,
                status: format!("{:?}", t.status),
                created_at: t.created_at,
            })
            .collect())
    }

    /// Get cluster nodes
    async fn cluster_nodes(&self, ctx: &Context<'_>) -> Result<Vec<ClusterNode>> {
        let state = ctx.data::<Arc<AppState>>()?;

        let nodes = state.cluster_manager.list_nodes();

        Ok(nodes
            .into_iter()
            .map(|n| ClusterNode {
                id: n.id.0.clone(),
                address: n.address,
                status: format!("{:?}", n.status),
                last_heartbeat: n.last_heartbeat,
            })
            .collect())
    }

    /// Get metrics summary
    async fn metrics_summary(&self, ctx: &Context<'_>) -> Result<MetricsSummary> {
        let state = ctx.data::<Arc<AppState>>()?;

        // Get all workflow stats and aggregate
        let all_stats = state.analytics.get_all_workflow_stats();

        let total_runs: u64 = all_stats.iter().map(|s| s.execution_count).sum();
        let successful_runs: u64 = all_stats.iter().map(|s| s.success_count).sum();
        let failed_runs: u64 = all_stats.iter().map(|s| s.failure_count).sum();
        let avg_duration_secs: f64 = if !all_stats.is_empty() {
            all_stats.iter().map(|s| s.avg_duration_secs).sum::<f64>() / all_stats.len() as f64
        } else {
            0.0
        };

        Ok(MetricsSummary {
            total_runs: total_runs as i64,
            successful_runs: successful_runs as i64,
            failed_runs: failed_runs as i64,
            avg_duration_ms: avg_duration_secs * 1000.0,
        })
    }

    /// Get system health status
    async fn system_health(&self, ctx: &Context<'_>) -> Result<SystemHealth> {
        let state = ctx.data::<Arc<AppState>>()?;

        // Calculate health from runs
        let all_stats = state.analytics.get_all_workflow_stats();
        let total_runs: u64 = all_stats.iter().map(|s| s.execution_count).sum();
        let successful_runs: u64 = all_stats.iter().map(|s| s.success_count).sum();
        let failed_runs: u64 = all_stats.iter().map(|s| s.failure_count).sum();

        let success_rate = if total_runs > 0 {
            (successful_runs as f64 / total_runs as f64) * 100.0
        } else {
            0.0
        };

        let overall_status = if success_rate >= 95.0 {
            "Healthy"
        } else if success_rate >= 80.0 {
            "Degraded"
        } else {
            "Unhealthy"
        };

        Ok(SystemHealth {
            overall_status: overall_status.to_string(),
            total_runs: total_runs as i32,
            successful_runs: successful_runs as i32,
            failed_runs: failed_runs as i32,
            success_rate,
        })
    }
}

/// System health status
#[derive(Clone, SimpleObject)]
pub struct SystemHealth {
    pub overall_status: String,
    pub total_runs: i32,
    pub successful_runs: i32,
    pub failed_runs: i32,
    pub success_rate: f64,
}

/// Mutation root
pub struct Mutation;

#[Object]
impl Mutation {
    /// Create a new workflow execution
    async fn create_run(&self, ctx: &Context<'_>, input: CreateRunInput) -> Result<Run> {
        let state = ctx.data::<Arc<AppState>>()?;

        // Placeholder implementation
        Ok(Run {
            id: uuid::Uuid::new_v4().to_string(),
            workflow_id: input.workflow_id,
            status: "Pending".to_string(),
            started_at: Utc::now(),
            completed_at: None,
            error: None,
        })
    }

    /// Register a new tenant
    async fn register_tenant(&self, ctx: &Context<'_>, input: RegisterTenantInput) -> Result<Tenant> {
        let state = ctx.data::<Arc<AppState>>()?;

        let quota = tenant::TenantQuota {
            max_concurrent_workflows: input.max_workflows.map(|v| v as u32),
            max_workflows_per_day: input.max_workflows.map(|v| v as u32),
            max_routines: input.max_routines.map(|v| v as u32),
            max_storage_bytes: input.max_storage_mb.map(|v| (v as u64) * 1024 * 1024),
            max_api_requests_per_minute: input.max_api_calls_per_hour.map(|v| (v as u32) / 60),
        };

        let now = chrono::Utc::now();
        let tenant = tenant::Tenant {
            id: tenant::TenantId(uuid::Uuid::new_v4().to_string()),
            name: input.name.clone(),
            description: "Created via GraphQL".to_string(),
            quota,
            settings: tenant::TenantSettings::default(),
            status: tenant::TenantStatus::Active,
            created_at: now,
            updated_at: now,
        };

        state.tenant_manager.register_tenant(tenant.clone())?;

        Ok(Tenant {
            id: tenant.id.0.clone(),
            name: tenant.name,
            status: format!("{:?}", tenant.status),
            created_at: tenant.created_at,
        })
    }

    /// Suspend a tenant
    async fn suspend_tenant(&self, ctx: &Context<'_>, id: String) -> Result<Tenant> {
        let state = ctx.data::<Arc<AppState>>()?;

        state.tenant_manager.suspend_tenant(&tenant::TenantId(id.clone()))?;

        let tenant = state
            .tenant_manager
            .get_tenant(&tenant::TenantId(id.clone()))
            .ok_or_else(|| Error::new("Tenant not found"))?;

        Ok(Tenant {
            id: tenant.id.0.clone(),
            name: tenant.name,
            status: format!("{:?}", tenant.status),
            created_at: tenant.created_at,
        })
    }
}

/// Input for creating a run
#[derive(InputObject)]
pub struct CreateRunInput {
    pub workflow_id: String,
    pub inputs: Option<serde_json::Value>,
}

/// Input for registering a tenant
#[derive(InputObject)]
pub struct RegisterTenantInput {
    pub name: String,
    pub max_workflows: Option<i32>,
    pub max_routines: Option<i32>,
    pub max_storage_mb: Option<i32>,
    pub max_api_calls_per_hour: Option<i32>,
}

/// Subscription root for real-time updates
pub struct Subscription;

#[Subscription]
impl Subscription {
    /// Subscribe to run status updates
    async fn run_updates(&self, _ctx: &Context<'_>) -> impl Stream<Item = Run> {
        // Create a stream that emits run updates
        // For now, this is a placeholder that emits every 5 seconds
        async_stream::stream! {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                yield Run {
                    id: uuid::Uuid::new_v4().to_string(),
                    workflow_id: "sample-workflow".to_string(),
                    status: "Running".to_string(),
                    started_at: Utc::now(),
                    completed_at: None,
                    error: None,
                };
            }
        }
    }

    /// Subscribe to audit log events
    async fn audit_events(&self, _ctx: &Context<'_>) -> impl Stream<Item = AuditEntry> {
        async_stream::stream! {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                yield AuditEntry {
                    id: uuid::Uuid::new_v4().to_string(),
                    timestamp: Utc::now(),
                    category: "SystemEvent".to_string(),
                    severity: "Info".to_string(),
                    user_id: None,
                    tenant_id: None,
                };
            }
        }
    }

    /// Subscribe to system metrics
    async fn metrics_updates(&self, ctx: &Context<'_>) -> impl Stream<Item = MetricsSummary> {
        // Clone the state before entering the stream to avoid lifetime issues
        let state_opt = ctx.data::<Arc<AppState>>().ok().cloned();

        async_stream::stream! {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));

            loop {
                interval.tick().await;

                if let Some(state) = &state_opt {
                    // Get all workflow stats and aggregate
                    let all_stats = state.analytics.get_all_workflow_stats();

                    let total_runs: u64 = all_stats.iter().map(|s| s.execution_count).sum();
                    let successful_runs: u64 = all_stats.iter().map(|s| s.success_count).sum();
                    let failed_runs: u64 = all_stats.iter().map(|s| s.failure_count).sum();
                    let avg_duration_secs: f64 = if !all_stats.is_empty() {
                        all_stats.iter().map(|s| s.avg_duration_secs).sum::<f64>() / all_stats.len() as f64
                    } else {
                        0.0
                    };

                    yield MetricsSummary {
                        total_runs: total_runs as i64,
                        successful_runs: successful_runs as i64,
                        failed_runs: failed_runs as i64,
                        avg_duration_ms: avg_duration_secs * 1000.0,
                    };
                }
            }
        }
    }
}

/// Build the GraphQL schema
pub type ShiiooSchema = Schema<Query, Mutation, Subscription>;

pub fn build_schema(state: Arc<AppState>) -> ShiiooSchema {
    Schema::build(Query, Mutation, Subscription)
        .data(state)
        .finish()
}
