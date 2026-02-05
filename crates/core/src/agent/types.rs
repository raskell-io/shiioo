//! Core agent types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

use crate::types::TeamId;

use super::{
    AgentBudgets, AgentPolicies, AgentSkills, ArchetypeId, McpBinding, SecretGrant,
};

/// Unique identifier for an agent.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub String);

impl AgentId {
    /// Create a new agent ID.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Generate a new random agent ID.
    pub fn generate() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for AgentId {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for AgentId {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

/// Status of an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    /// Agent is active and can execute tasks.
    Active,
    /// Agent is paused (won't be assigned new work).
    Paused,
    /// Agent is suspended (policy violation, budget exceeded).
    Suspended,
    /// Agent is archived (soft delete).
    Archived,
}

impl Default for AgentStatus {
    fn default() -> Self {
        Self::Active
    }
}

impl fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Paused => write!(f, "paused"),
            Self::Suspended => write!(f, "suspended"),
            Self::Archived => write!(f, "archived"),
        }
    }
}

/// Where an agent sits in the organizational structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentOrganization {
    /// Team membership.
    pub team: TeamId,

    /// Direct supervisor (for escalation).
    pub reports_to: Option<AgentId>,

    /// Agents this agent supervises.
    pub supervises: Vec<AgentId>,

    /// Peer agents (can delegate to).
    pub peers: Vec<AgentId>,
}

impl AgentOrganization {
    /// Create a new agent organization placement.
    pub fn new(team: TeamId) -> Self {
        Self {
            team,
            reports_to: None,
            supervises: Vec::new(),
            peers: Vec::new(),
        }
    }

    /// Set the supervisor.
    pub fn with_supervisor(mut self, supervisor: AgentId) -> Self {
        self.reports_to = Some(supervisor);
        self
    }

    /// Add supervised agents.
    pub fn with_supervises(mut self, agents: Vec<AgentId>) -> Self {
        self.supervises = agents;
        self
    }

    /// Add peer agents.
    pub fn with_peers(mut self, peers: Vec<AgentId>) -> Self {
        self.peers = peers;
        self
    }
}

/// A virtual employee in the enterprise.
///
/// An Agent combines identity, capabilities (skills), governance (policies),
/// and integrations (MCP bindings) into a single coherent primitive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    /// Unique identifier.
    pub id: AgentId,

    /// Human-readable name.
    pub name: String,

    /// What this agent does (used for routing/discovery).
    pub description: String,

    /// Agent archetype (optional inheritance).
    pub archetype: Option<ArchetypeId>,

    /// Organizational placement.
    pub organization: AgentOrganization,

    /// Skills this agent has (Agent Skills format).
    pub skills: AgentSkills,

    /// Governance policies.
    pub policies: AgentPolicies,

    /// MCP tool bindings.
    pub mcp_bindings: Vec<McpBinding>,

    /// Secret access grants.
    pub secret_grants: Vec<SecretGrant>,

    /// Resource budgets.
    pub budgets: AgentBudgets,

    /// Agent status.
    pub status: AgentStatus,

    /// When the agent was created.
    pub created_at: DateTime<Utc>,

    /// When the agent was last updated.
    pub updated_at: DateTime<Utc>,

    /// Who created this agent.
    pub created_by: String,
}

impl Agent {
    /// Create a new agent builder.
    pub fn builder(id: impl Into<AgentId>, name: impl Into<String>) -> AgentBuilder {
        AgentBuilder::new(id.into(), name.into())
    }

    /// Check if the agent is active.
    pub fn is_active(&self) -> bool {
        self.status == AgentStatus::Active
    }

    /// Check if the agent can be assigned work.
    pub fn can_accept_work(&self) -> bool {
        matches!(self.status, AgentStatus::Active)
    }

    /// Get the agent's supervisor, if any.
    pub fn supervisor(&self) -> Option<&AgentId> {
        self.organization.reports_to.as_ref()
    }

    /// Check if this agent supervises another agent.
    pub fn supervises(&self, agent_id: &AgentId) -> bool {
        self.organization.supervises.contains(agent_id)
    }

    /// Check if another agent is a peer.
    pub fn is_peer(&self, agent_id: &AgentId) -> bool {
        self.organization.peers.contains(agent_id)
    }
}

/// Builder for creating agents.
pub struct AgentBuilder {
    id: AgentId,
    name: String,
    description: String,
    archetype: Option<ArchetypeId>,
    organization: Option<AgentOrganization>,
    skills: AgentSkills,
    policies: AgentPolicies,
    mcp_bindings: Vec<McpBinding>,
    secret_grants: Vec<SecretGrant>,
    budgets: AgentBudgets,
    status: AgentStatus,
    created_by: String,
}

impl AgentBuilder {
    /// Create a new agent builder.
    pub fn new(id: AgentId, name: String) -> Self {
        Self {
            id,
            name,
            description: String::new(),
            archetype: None,
            organization: None,
            skills: AgentSkills::default(),
            policies: AgentPolicies::default(),
            mcp_bindings: Vec::new(),
            secret_grants: Vec::new(),
            budgets: AgentBudgets::default(),
            status: AgentStatus::Active,
            created_by: String::from("system"),
        }
    }

    /// Set the agent description.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Set the agent archetype.
    pub fn archetype(mut self, archetype: ArchetypeId) -> Self {
        self.archetype = Some(archetype);
        self
    }

    /// Set the organizational placement.
    pub fn organization(mut self, organization: AgentOrganization) -> Self {
        self.organization = Some(organization);
        self
    }

    /// Set the agent skills.
    pub fn skills(mut self, skills: AgentSkills) -> Self {
        self.skills = skills;
        self
    }

    /// Set the agent policies.
    pub fn policies(mut self, policies: AgentPolicies) -> Self {
        self.policies = policies;
        self
    }

    /// Add MCP bindings.
    pub fn mcp_bindings(mut self, bindings: Vec<McpBinding>) -> Self {
        self.mcp_bindings = bindings;
        self
    }

    /// Add secret grants.
    pub fn secret_grants(mut self, grants: Vec<SecretGrant>) -> Self {
        self.secret_grants = grants;
        self
    }

    /// Set the agent budgets.
    pub fn budgets(mut self, budgets: AgentBudgets) -> Self {
        self.budgets = budgets;
        self
    }

    /// Set the agent status.
    pub fn status(mut self, status: AgentStatus) -> Self {
        self.status = status;
        self
    }

    /// Set who created this agent.
    pub fn created_by(mut self, created_by: impl Into<String>) -> Self {
        self.created_by = created_by.into();
        self
    }

    /// Build the agent.
    ///
    /// # Panics
    ///
    /// Panics if organization is not set. Use `build_with_defaults` for a
    /// version that provides default values.
    pub fn build(self) -> Agent {
        let now = Utc::now();
        Agent {
            id: self.id,
            name: self.name,
            description: self.description,
            archetype: self.archetype,
            organization: self
                .organization
                .expect("organization is required; use build_with_defaults() for defaults"),
            skills: self.skills,
            policies: self.policies,
            mcp_bindings: self.mcp_bindings,
            secret_grants: self.secret_grants,
            budgets: self.budgets,
            status: self.status,
            created_at: now,
            updated_at: now,
            created_by: self.created_by,
        }
    }

    /// Build the agent with default values for optional fields.
    pub fn build_with_defaults(self, default_team: TeamId) -> Agent {
        let now = Utc::now();
        Agent {
            id: self.id,
            name: self.name,
            description: self.description,
            archetype: self.archetype,
            organization: self
                .organization
                .unwrap_or_else(|| AgentOrganization::new(default_team)),
            skills: self.skills,
            policies: self.policies,
            mcp_bindings: self.mcp_bindings,
            secret_grants: self.secret_grants,
            budgets: self.budgets,
            status: self.status,
            created_at: now,
            updated_at: now,
            created_by: self.created_by,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_id_creation() {
        let id = AgentId::new("eng-alice");
        assert_eq!(id.0, "eng-alice");
        assert_eq!(id.to_string(), "eng-alice");
    }

    #[test]
    fn test_agent_id_from_str() {
        let id: AgentId = "eng-bob".into();
        assert_eq!(id.0, "eng-bob");
    }

    #[test]
    fn test_agent_status_default() {
        assert_eq!(AgentStatus::default(), AgentStatus::Active);
    }

    #[test]
    fn test_agent_organization() {
        let org = AgentOrganization::new(TeamId::new("platform"))
            .with_supervisor(AgentId::new("lead-bob"))
            .with_peers(vec![AgentId::new("eng-charlie")]);

        assert_eq!(org.team.0, "platform");
        assert_eq!(org.reports_to, Some(AgentId::new("lead-bob")));
        assert_eq!(org.peers.len(), 1);
    }

    #[test]
    fn test_agent_builder() {
        let agent = Agent::builder("eng-alice", "Alice")
            .description("Software engineer")
            .organization(AgentOrganization::new(TeamId::new("platform")))
            .created_by("admin")
            .build();

        assert_eq!(agent.id.0, "eng-alice");
        assert_eq!(agent.name, "Alice");
        assert_eq!(agent.description, "Software engineer");
        assert!(agent.is_active());
        assert!(agent.can_accept_work());
    }

    #[test]
    fn test_agent_supervisor_relationship() {
        let agent = Agent::builder("eng-alice", "Alice")
            .organization(
                AgentOrganization::new(TeamId::new("platform"))
                    .with_supervisor(AgentId::new("lead-bob"))
                    .with_supervises(vec![AgentId::new("intern-dave")]),
            )
            .build();

        assert_eq!(agent.supervisor(), Some(&AgentId::new("lead-bob")));
        assert!(agent.supervises(&AgentId::new("intern-dave")));
        assert!(!agent.supervises(&AgentId::new("eng-charlie")));
    }
}
