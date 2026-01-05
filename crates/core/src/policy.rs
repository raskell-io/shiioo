// Policy engine for governance and authorization (will be implemented in Phase 2)

use crate::types::{PolicySpec, RoleId};
use anyhow::Result;

/// Policy decision
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny { reason: String },
    RequiresApproval { approvers: Vec<String> },
}

/// Policy engine trait
#[async_trait::async_trait]
pub trait PolicyEngine: Send + Sync {
    /// Check if a tool call is allowed for a role
    async fn check_tool_call(
        &self,
        role: &RoleId,
        tool_id: &str,
        parameters: &serde_json::Value,
    ) -> Result<PolicyDecision>;

    /// Check if a configuration change is allowed
    async fn check_config_change(
        &self,
        proposed_by: &str,
        diff: &crate::types::ConfigDiff,
    ) -> Result<PolicyDecision>;

    /// Load policies
    async fn load_policies(&self, policies: Vec<PolicySpec>) -> Result<()>;
}
