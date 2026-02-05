//! Migration API handlers.

use super::ApiResult;
use crate::config::AppState;
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use shiioo_core::{
    agent::{FullMigrationReport, MigrationConfig, MigrationReport, MigrationRunner},
    storage::AgentStore,
    types::{Person, PersonId, PolicyId, PolicyRule, PolicySpec, RoleBudgets, RoleId, RoleSpec, TeamId},
};
use std::sync::Arc;

/// Run a full migration from legacy types
pub async fn run_migration(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RunMigrationRequest>,
) -> ApiResult<Json<RunMigrationResponse>> {
    let config = MigrationConfig {
        continue_on_error: req.continue_on_error.unwrap_or(true),
        auto_store: !req.dry_run.unwrap_or(false),
        dry_run: req.dry_run.unwrap_or(false),
    };

    let runner = MigrationRunner::new(state.agent_store.clone(), config);

    let report = runner
        .run_full_migration(&req.roles, &req.persons, &req.policies)
        .await;

    Ok(Json(RunMigrationResponse {
        success: report.is_successful(),
        total_migrated: report.total_migrated(),
        total_failed: report.total_failed(),
        report,
    }))
}

#[derive(Debug, Deserialize)]
pub struct RunMigrationRequest {
    pub roles: Vec<RoleSpec>,
    pub persons: Vec<Person>,
    pub policies: Vec<PolicySpec>,
    pub dry_run: Option<bool>,
    pub continue_on_error: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct RunMigrationResponse {
    pub success: bool,
    pub total_migrated: usize,
    pub total_failed: usize,
    pub report: FullMigrationReport,
}

/// Migrate roles to archetypes
pub async fn migrate_roles(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MigrateRolesRequest>,
) -> ApiResult<Json<MigrateRolesResponse>> {
    let config = MigrationConfig {
        continue_on_error: req.continue_on_error.unwrap_or(true),
        auto_store: !req.dry_run.unwrap_or(false),
        dry_run: req.dry_run.unwrap_or(false),
    };

    let runner = MigrationRunner::new(state.agent_store.clone(), config);
    let report = runner.migrate_roles(&req.roles).await;

    Ok(Json(MigrateRolesResponse {
        success: report.failed == 0,
        report,
    }))
}

#[derive(Debug, Deserialize)]
pub struct MigrateRolesRequest {
    pub roles: Vec<RoleSpec>,
    pub dry_run: Option<bool>,
    pub continue_on_error: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct MigrateRolesResponse {
    pub success: bool,
    pub report: MigrationReport,
}

/// Migrate persons to agents
pub async fn migrate_persons(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MigratePersonsRequest>,
) -> ApiResult<Json<MigratePersonsResponse>> {
    let config = MigrationConfig {
        continue_on_error: req.continue_on_error.unwrap_or(true),
        auto_store: !req.dry_run.unwrap_or(false),
        dry_run: req.dry_run.unwrap_or(false),
    };

    let runner = MigrationRunner::new(state.agent_store.clone(), config);

    // If role mappings are provided, register them first
    if let Some(mappings) = &req.role_mappings {
        for (role_id, archetype_id) in mappings {
            runner
                .register_role_mapping(role_id, shiioo_core::agent::ArchetypeId::new(archetype_id))
                .await;
        }
    }

    let report = runner.migrate_persons(&req.persons).await;

    Ok(Json(MigratePersonsResponse {
        success: report.failed == 0,
        report,
    }))
}

#[derive(Debug, Deserialize)]
pub struct MigratePersonsRequest {
    pub persons: Vec<Person>,
    /// Optional mapping from role IDs to archetype IDs.
    /// If not provided, persons won't have archetypes assigned.
    pub role_mappings: Option<std::collections::HashMap<String, String>>,
    pub dry_run: Option<bool>,
    pub continue_on_error: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct MigratePersonsResponse {
    pub success: bool,
    pub report: MigrationReport,
}

/// Preview a migration without executing it (alias for dry_run=true)
pub async fn preview_migration(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PreviewMigrationRequest>,
) -> ApiResult<Json<PreviewMigrationResponse>> {
    let config = MigrationConfig {
        continue_on_error: true,
        auto_store: false,
        dry_run: true,
    };

    let runner = MigrationRunner::new(state.agent_store.clone(), config);

    let report = runner
        .run_full_migration(&req.roles, &req.persons, &req.policies)
        .await;

    Ok(Json(PreviewMigrationResponse {
        would_migrate_roles: req.roles.len(),
        would_migrate_persons: req.persons.len(),
        expected_archetypes: req.roles.len(),
        expected_agents: req.persons.len(),
        potential_issues: report
            .roles
            .errors
            .into_iter()
            .chain(report.persons.errors)
            .map(|e| format!("{} ({}): {}", e.source_type, e.source_id, e.error))
            .collect(),
        policy_warnings: report.policy_warnings,
    }))
}

#[derive(Debug, Deserialize)]
pub struct PreviewMigrationRequest {
    pub roles: Vec<RoleSpec>,
    pub persons: Vec<Person>,
    pub policies: Vec<PolicySpec>,
}

#[derive(Debug, Serialize)]
pub struct PreviewMigrationResponse {
    pub would_migrate_roles: usize,
    pub would_migrate_persons: usize,
    pub expected_archetypes: usize,
    pub expected_agents: usize,
    pub potential_issues: Vec<String>,
    pub policy_warnings: Vec<String>,
}
