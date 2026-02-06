//! Migration tooling for converting legacy types to the new Agent model.
//!
//! This module provides:
//! - Adapters for converting legacy `RoleSpec`, `Person`, and `PolicySpec` to new types
//! - `MigrationRunner` for executing bulk migrations
//! - Migration status tracking and reporting

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use super::{
    Agent, AgentId, AgentOrganization, AgentPolicies, ArchetypeId, ApprovalRule, ApprovalTrigger, ApproverSpec, PolicyRule as AgentPolicyRule, PolicyScope,
    TimeoutAction,
};
use crate::storage::AgentStore;
use crate::types::{Person, PolicyRule, PolicySpec, RoleSpec};
#[cfg(test)]
use crate::types::{PersonId, RoleBudgets, RoleId, TeamId};

/// Migration result for a single item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationResult<T> {
    /// The migrated item (if successful).
    pub item: Option<T>,
    /// Source ID.
    pub source_id: String,
    /// Source type name.
    pub source_type: String,
    /// Whether the migration was successful.
    pub success: bool,
    /// Error message (if failed).
    pub error: Option<String>,
    /// Warnings generated during migration.
    pub warnings: Vec<String>,
    /// Timestamp of the migration.
    pub migrated_at: DateTime<Utc>,
}

impl<T> MigrationResult<T> {
    /// Create a successful migration result.
    pub fn success(item: T, source_id: String, source_type: &str) -> Self {
        Self {
            item: Some(item),
            source_id,
            source_type: source_type.to_string(),
            success: true,
            error: None,
            warnings: Vec::new(),
            migrated_at: Utc::now(),
        }
    }

    /// Create a successful migration result with warnings.
    pub fn success_with_warnings(item: T, source_id: String, source_type: &str, warnings: Vec<String>) -> Self {
        Self {
            item: Some(item),
            source_id,
            source_type: source_type.to_string(),
            success: true,
            error: None,
            warnings,
            migrated_at: Utc::now(),
        }
    }

    /// Create a failed migration result.
    pub fn failure(source_id: String, source_type: &str, error: String) -> Self {
        Self {
            item: None,
            source_id,
            source_type: source_type.to_string(),
            success: false,
            error: Some(error),
            warnings: Vec::new(),
            migrated_at: Utc::now(),
        }
    }
}

/// Batch migration report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationReport {
    /// Total items attempted.
    pub total: usize,
    /// Successfully migrated items.
    pub successful: usize,
    /// Failed items.
    pub failed: usize,
    /// Items with warnings.
    pub with_warnings: usize,
    /// Start time.
    pub started_at: DateTime<Utc>,
    /// End time.
    pub completed_at: DateTime<Utc>,
    /// Individual errors.
    pub errors: Vec<MigrationError>,
}

/// A migration error entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationError {
    pub source_id: String,
    pub source_type: String,
    pub error: String,
}

// ============================================================================
// Person → Agent Adapter
// ============================================================================

/// Adapter for converting legacy `Person` to `Agent`.
pub struct PersonAdapter;

impl PersonAdapter {
    /// Convert a Person to an Agent.
    ///
    /// This creates an agent with:
    /// - ID derived from person ID
    /// - Basic organization info from person
    /// - Archetype reference from role (if role exists as archetype)
    /// - Default skills and policies (inherited from archetype)
    pub fn to_agent(person: &Person, archetype_id: Option<&ArchetypeId>) -> MigrationResult<Agent> {
        let mut warnings = Vec::new();

        // Map person ID to agent ID
        let agent_id = AgentId::new(format!("agent-{}", person.id.0));

        // Map reports_to from PersonId to AgentId
        let reports_to = person.reports_to.as_ref().map(|pid| {
            AgentId::new(format!("agent-{}", pid.0))
        });

        // Create organization
        let organization = AgentOrganization {
            team: person.team.clone(),
            reports_to,
            supervises: Vec::new(), // Will be populated in a second pass
            peers: Vec::new(),      // Will be populated in a second pass
        };

        // Build the agent
        let mut builder = Agent::builder(agent_id.clone(), &person.name)
            .description(format!("Migrated from Person: {}", person.email))
            .organization(organization);

        // Set archetype if provided
        if let Some(arch_id) = archetype_id {
            builder = builder.archetype(arch_id.clone());
        } else {
            warnings.push(format!(
                "No archetype found for role '{}', agent will use default skills/policies",
                person.role.0
            ));
        }

        // Convert can_approve to approval-related policies
        if !person.can_approve.is_empty() {
            // Person can approve certain things, but as an agent this translates to
            // being referenced as an approver, not having approval rules themselves
            warnings.push(format!(
                "Person's can_approve list ({:?}) should be configured in other agents' policies",
                person.can_approve
            ));
        }

        let agent = builder.build();

        if warnings.is_empty() {
            MigrationResult::success(agent, person.id.0.clone(), "Person")
        } else {
            MigrationResult::success_with_warnings(agent, person.id.0.clone(), "Person", warnings)
        }
    }

    /// Batch convert multiple persons to agents.
    pub fn batch_to_agents(
        persons: &[Person],
        role_to_archetype: &HashMap<String, ArchetypeId>,
    ) -> Vec<MigrationResult<Agent>> {
        persons
            .iter()
            .map(|person| {
                let archetype_id = role_to_archetype.get(&person.role.0);
                Self::to_agent(person, archetype_id)
            })
            .collect()
    }

    /// Second pass: populate supervises and peers relationships.
    pub fn populate_relationships(
        agents: &mut [Agent],
        persons: &[Person],
    ) {
        // Build a map of person_id -> agent index
        let person_to_agent: HashMap<String, usize> = agents
            .iter()
            .enumerate()
            .map(|(idx, agent)| {
                // Extract original person ID from agent ID
                let person_id = agent.id.0.strip_prefix("agent-").unwrap_or(&agent.id.0);
                (person_id.to_string(), idx)
            })
            .collect();

        // For each person, find who reports to them (their supervisees)
        for person in persons {
            let person_id = &person.id.0;

            // Find the agent for this person
            if let Some(&agent_idx) = person_to_agent.get(person_id) {
                // Find all persons who report to this person
                let supervisees: Vec<AgentId> = persons
                    .iter()
                    .filter(|p| p.reports_to.as_ref().map(|r| &r.0) == Some(person_id))
                    .filter_map(|p| person_to_agent.get(&p.id.0).map(|_| AgentId::new(format!("agent-{}", p.id.0))))
                    .collect();

                agents[agent_idx].organization.supervises = supervisees;

                // Find peers (same team, same reports_to)
                let peers: Vec<AgentId> = persons
                    .iter()
                    .filter(|p| {
                        p.id.0 != *person_id
                            && p.team.0 == person.team.0
                            && p.reports_to == person.reports_to
                    })
                    .filter_map(|p| person_to_agent.get(&p.id.0).map(|_| AgentId::new(format!("agent-{}", p.id.0))))
                    .collect();

                agents[agent_idx].organization.peers = peers;
            }
        }
    }
}

// ============================================================================
// PolicySpec → AgentPolicies Adapter
// ============================================================================

/// Adapter for converting legacy `PolicySpec` to `AgentPolicies`.
pub struct PolicySpecAdapter;

impl PolicySpecAdapter {
    /// Convert a PolicySpec to AgentPolicies.
    pub fn to_agent_policies(policy_spec: &PolicySpec) -> MigrationResult<AgentPolicies> {
        let mut warnings = Vec::new();
        let mut policies = AgentPolicies::new();

        for rule in &policy_spec.rules {
            match rule {
                PolicyRule::DenyPath { patterns } => {
                    let deny_rule = AgentPolicyRule::new(
                        format!("{}-deny-paths", policy_spec.id.0),
                        format!("Deny access to paths: {:?}", patterns),
                        PolicyScope::Resources {
                            patterns: patterns.clone(),
                        },
                    );
                    policies = policies.with_deny(deny_rule);
                }
                PolicyRule::AllowDomain { domains } => {
                    // Convert domains to resource patterns
                    let patterns: Vec<String> = domains
                        .iter()
                        .map(|d| format!("**/{}", d))
                        .collect();
                    let allow_rule = AgentPolicyRule::new(
                        format!("{}-allow-domains", policy_spec.id.0),
                        format!("Allow access to domains: {:?}", domains),
                        PolicyScope::Resources { patterns },
                    );
                    policies = policies.with_allow(allow_rule);
                }
                PolicyRule::RequireApproval { tool_ids } => {
                    let approval_rule = ApprovalRule::new(
                        format!("{}-require-approval", policy_spec.id.0),
                        ApprovalTrigger::Tools {
                            tool_ids: tool_ids.clone(),
                        },
                        ApproverSpec::supervisor(),
                    )
                    .with_timeout(chrono::Duration::hours(1), TimeoutAction::Deny);
                    policies = policies.with_approval(approval_rule);
                }
                PolicyRule::EnforceEnvironment { environment } => {
                    // This is more of a runtime constraint, add as a warning
                    warnings.push(format!(
                        "EnforceEnvironment rule for '{}' should be handled at runtime",
                        environment
                    ));
                }
            }
        }

        if warnings.is_empty() {
            MigrationResult::success(policies, policy_spec.id.0.clone(), "PolicySpec")
        } else {
            MigrationResult::success_with_warnings(
                policies,
                policy_spec.id.0.clone(),
                "PolicySpec",
                warnings,
            )
        }
    }
}

// ============================================================================
// Migration Runner
// ============================================================================

/// Configuration for the migration runner.
#[derive(Debug, Clone)]
pub struct MigrationConfig {
    /// Whether to continue on errors.
    pub continue_on_error: bool,
    /// Whether to store migrated items immediately.
    pub auto_store: bool,
    /// Dry run mode (validate without storing).
    pub dry_run: bool,
}

impl Default for MigrationConfig {
    fn default() -> Self {
        Self {
            continue_on_error: true,
            auto_store: true,
            dry_run: false,
        }
    }
}

/// Migration runner for executing bulk migrations.
pub struct MigrationRunner<S: AgentStore> {
    store: Arc<S>,
    config: MigrationConfig,
    /// Mapping from legacy RoleId to new ArchetypeId.
    role_to_archetype: RwLock<HashMap<String, ArchetypeId>>,
}

impl<S: AgentStore> MigrationRunner<S> {
    /// Create a new migration runner.
    pub fn new(store: Arc<S>, config: MigrationConfig) -> Self {
        Self {
            store,
            config,
            role_to_archetype: RwLock::new(HashMap::new()),
        }
    }

    /// Register a role-to-archetype mapping.
    pub async fn register_role_mapping(&self, role_id: &str, archetype_id: ArchetypeId) {
        let mut map = self.role_to_archetype.write().await;
        map.insert(role_id.to_string(), archetype_id);
    }

    /// Migrate roles to archetypes.
    ///
    /// Uses `RoleSpecAdapter` from `archetype_manager`.
    pub async fn migrate_roles(&self, roles: &[RoleSpec]) -> MigrationReport {
        use super::RoleSpecAdapter;

        let started_at = Utc::now();
        let mut successful = 0;
        let mut failed = 0;
        let with_warnings = 0;
        let mut errors = Vec::new();

        for role in roles {
            let archetype = RoleSpecAdapter::to_archetype(role);

            // Register the mapping
            self.register_role_mapping(&role.id.0, archetype.id.clone()).await;

            if !self.config.dry_run && self.config.auto_store {
                match self.store.store_archetype(&archetype).await {
                    Ok(_) => {
                        successful += 1;
                        tracing::info!(
                            "Migrated role '{}' to archetype '{}'",
                            role.id.0,
                            archetype.id.0
                        );
                    }
                    Err(e) => {
                        failed += 1;
                        errors.push(MigrationError {
                            source_id: role.id.0.clone(),
                            source_type: "RoleSpec".to_string(),
                            error: e.to_string(),
                        });
                        if !self.config.continue_on_error {
                            break;
                        }
                    }
                }
            } else {
                successful += 1;
            }
        }

        MigrationReport {
            total: roles.len(),
            successful,
            failed,
            with_warnings,
            started_at,
            completed_at: Utc::now(),
            errors,
        }
    }

    /// Migrate persons to agents.
    pub async fn migrate_persons(&self, persons: &[Person]) -> MigrationReport {
        let started_at = Utc::now();
        let mut successful = 0;
        let mut failed = 0;
        let mut with_warnings = 0;
        let mut errors = Vec::new();

        // Get the role-to-archetype mapping
        let role_map = self.role_to_archetype.read().await;

        // First pass: create agents
        let results = PersonAdapter::batch_to_agents(persons, &role_map);
        let mut agents: Vec<Agent> = Vec::new();

        for result in results {
            if result.success {
                if !result.warnings.is_empty() {
                    with_warnings += 1;
                    for warning in &result.warnings {
                        tracing::warn!(
                            "Migration warning for Person '{}': {}",
                            result.source_id,
                            warning
                        );
                    }
                }
                if let Some(agent) = result.item {
                    agents.push(agent);
                }
            } else {
                failed += 1;
                if let Some(error) = result.error {
                    errors.push(MigrationError {
                        source_id: result.source_id,
                        source_type: "Person".to_string(),
                        error,
                    });
                }
                if !self.config.continue_on_error {
                    break;
                }
            }
        }

        // Second pass: populate relationships
        PersonAdapter::populate_relationships(&mut agents, persons);

        // Store agents
        for agent in agents {
            if !self.config.dry_run && self.config.auto_store {
                match self.store.store_agent(&agent).await {
                    Ok(_) => {
                        successful += 1;
                        tracing::info!(
                            "Migrated person to agent '{}'",
                            agent.id.0
                        );
                    }
                    Err(e) => {
                        failed += 1;
                        errors.push(MigrationError {
                            source_id: agent.id.0.clone(),
                            source_type: "Person".to_string(),
                            error: e.to_string(),
                        });
                        if !self.config.continue_on_error {
                            break;
                        }
                    }
                }
            } else {
                successful += 1;
            }
        }

        MigrationReport {
            total: persons.len(),
            successful,
            failed,
            with_warnings,
            started_at,
            completed_at: Utc::now(),
            errors,
        }
    }

    /// Run a full migration from legacy types.
    pub async fn run_full_migration(
        &self,
        roles: &[RoleSpec],
        persons: &[Person],
        policies: &[PolicySpec],
    ) -> FullMigrationReport {
        let started_at = Utc::now();

        tracing::info!("Starting full migration: {} roles, {} persons, {} policies",
            roles.len(), persons.len(), policies.len());

        // Step 1: Migrate roles to archetypes
        let roles_report = self.migrate_roles(roles).await;
        tracing::info!(
            "Roles migration: {}/{} successful",
            roles_report.successful,
            roles_report.total
        );

        // Step 2: Migrate persons to agents
        let persons_report = self.migrate_persons(persons).await;
        tracing::info!(
            "Persons migration: {}/{} successful",
            persons_report.successful,
            persons_report.total
        );

        // Step 3: Note about policies (they're applied to archetypes/agents, not standalone)
        let policies_warnings: Vec<String> = policies
            .iter()
            .map(|p| format!(
                "PolicySpec '{}' should be applied to specific archetypes/agents",
                p.id.0
            ))
            .collect();

        for warning in &policies_warnings {
            tracing::warn!("{}", warning);
        }

        FullMigrationReport {
            roles: roles_report,
            persons: persons_report,
            policy_warnings: policies_warnings,
            started_at,
            completed_at: Utc::now(),
        }
    }
}

/// Report for a full migration run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullMigrationReport {
    /// Roles migration report.
    pub roles: MigrationReport,
    /// Persons migration report.
    pub persons: MigrationReport,
    /// Policy migration warnings (policies are applied to agents/archetypes).
    pub policy_warnings: Vec<String>,
    /// Start time.
    pub started_at: DateTime<Utc>,
    /// End time.
    pub completed_at: DateTime<Utc>,
}

impl FullMigrationReport {
    /// Check if the full migration was successful.
    pub fn is_successful(&self) -> bool {
        self.roles.failed == 0 && self.persons.failed == 0
    }

    /// Get total items migrated.
    pub fn total_migrated(&self) -> usize {
        self.roles.successful + self.persons.successful
    }

    /// Get total failures.
    pub fn total_failed(&self) -> usize {
        self.roles.failed + self.persons.failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::InMemoryAgentStore;

    fn create_test_role(id: &str, name: &str) -> RoleSpec {
        RoleSpec {
            id: RoleId::new(id),
            name: name.to_string(),
            description: format!("Test role: {}", name),
            prompt_template: "You are a test agent.".to_string(),
            allowed_tools: vec!["tool1".to_string(), "tool2".to_string()],
            budgets: RoleBudgets {
                daily_tokens: Some(1_000_000),
                daily_cost_cents: Some(5000),
            },
            requires_approval_for: vec!["deploy".to_string()],
        }
    }

    fn create_test_person(id: &str, name: &str, role_id: &str, team_id: &str) -> Person {
        Person {
            id: PersonId::new(id),
            name: name.to_string(),
            email: format!("{}@example.com", id),
            role: RoleId::new(role_id),
            team: TeamId::new(team_id),
            reports_to: None,
            can_approve: vec![],
        }
    }

    #[test]
    fn test_person_to_agent() {
        let person = create_test_person("alice", "Alice", "engineer", "platform");
        let archetype_id = ArchetypeId::new("engineer");

        let result = PersonAdapter::to_agent(&person, Some(&archetype_id));

        assert!(result.success);
        let agent = result.item.unwrap();
        assert_eq!(agent.id.0, "agent-alice");
        assert_eq!(agent.name, "Alice");
        assert_eq!(agent.organization.team.0, "platform");
        assert_eq!(agent.archetype, Some(archetype_id));
    }

    #[test]
    fn test_person_to_agent_no_archetype() {
        let person = create_test_person("bob", "Bob", "unknown-role", "platform");

        let result = PersonAdapter::to_agent(&person, None);

        assert!(result.success);
        assert!(!result.warnings.is_empty());
        assert!(result.warnings[0].contains("No archetype found"));
    }

    #[test]
    fn test_populate_relationships() {
        let mut persons = vec![
            create_test_person("alice", "Alice", "lead", "platform"),
            create_test_person("bob", "Bob", "engineer", "platform"),
            create_test_person("charlie", "Charlie", "engineer", "platform"),
        ];
        persons[1].reports_to = Some(PersonId::new("alice"));
        persons[2].reports_to = Some(PersonId::new("alice"));

        let role_map = HashMap::new();
        let results = PersonAdapter::batch_to_agents(&persons, &role_map);
        let mut agents: Vec<Agent> = results.into_iter().filter_map(|r| r.item).collect();

        PersonAdapter::populate_relationships(&mut agents, &persons);

        // Alice should supervise Bob and Charlie
        let alice = agents.iter().find(|a| a.id.0 == "agent-alice").unwrap();
        assert_eq!(alice.organization.supervises.len(), 2);

        // Bob and Charlie should be peers
        let bob = agents.iter().find(|a| a.id.0 == "agent-bob").unwrap();
        assert_eq!(bob.organization.peers.len(), 1);
        assert_eq!(bob.organization.peers[0].0, "agent-charlie");
    }

    #[test]
    fn test_policy_spec_to_agent_policies() {
        use crate::types::PolicyId;

        let policy_spec = PolicySpec {
            id: PolicyId("test-policy".to_string()),
            name: "Test Policy".to_string(),
            description: "A test policy".to_string(),
            rules: vec![
                PolicyRule::DenyPath {
                    patterns: vec!["**/secrets/**".to_string()],
                },
                PolicyRule::RequireApproval {
                    tool_ids: vec!["deploy".to_string()],
                },
            ],
        };

        let result = PolicySpecAdapter::to_agent_policies(&policy_spec);

        assert!(result.success);
        let policies = result.item.unwrap();
        assert_eq!(policies.deny.len(), 1);
        assert_eq!(policies.requires_approval.len(), 1);
    }

    #[tokio::test]
    async fn test_migration_runner_roles() {
        let store = Arc::new(InMemoryAgentStore::new());
        let config = MigrationConfig::default();
        let runner = MigrationRunner::new(store.clone(), config);

        let roles = vec![
            create_test_role("engineer", "Software Engineer"),
            create_test_role("lead", "Engineering Lead"),
        ];

        let report = runner.migrate_roles(&roles).await;

        assert_eq!(report.total, 2);
        assert_eq!(report.successful, 2);
        assert_eq!(report.failed, 0);

        // Verify archetypes were stored
        let archetypes = store.list_archetypes().await.unwrap();
        assert_eq!(archetypes.len(), 2);
    }

    #[tokio::test]
    async fn test_migration_runner_persons() {
        let store = Arc::new(InMemoryAgentStore::new());
        let config = MigrationConfig::default();
        let runner = MigrationRunner::new(store.clone(), config);

        // First migrate roles
        let roles = vec![create_test_role("engineer", "Software Engineer")];
        runner.migrate_roles(&roles).await;

        // Then migrate persons
        let persons = vec![
            create_test_person("alice", "Alice", "engineer", "platform"),
            create_test_person("bob", "Bob", "engineer", "platform"),
        ];

        let report = runner.migrate_persons(&persons).await;

        assert_eq!(report.total, 2);
        assert_eq!(report.successful, 2);
        assert_eq!(report.failed, 0);

        // Verify agents were stored
        let agents = store.list_agents().await.unwrap();
        assert_eq!(agents.len(), 2);
    }

    #[tokio::test]
    async fn test_full_migration() {
        let store = Arc::new(InMemoryAgentStore::new());
        let config = MigrationConfig::default();
        let runner = MigrationRunner::new(store.clone(), config);

        let roles = vec![
            create_test_role("engineer", "Software Engineer"),
            create_test_role("lead", "Engineering Lead"),
        ];

        let mut persons = vec![
            create_test_person("alice", "Alice", "lead", "platform"),
            create_test_person("bob", "Bob", "engineer", "platform"),
        ];
        persons[1].reports_to = Some(PersonId::new("alice"));

        let policies = vec![];

        let report = runner.run_full_migration(&roles, &persons, &policies).await;

        assert!(report.is_successful());
        assert_eq!(report.total_migrated(), 4); // 2 roles + 2 persons

        // Verify storage
        let archetypes = store.list_archetypes().await.unwrap();
        assert_eq!(archetypes.len(), 2);

        let agents = store.list_agents().await.unwrap();
        assert_eq!(agents.len(), 2);

        // Verify relationships were set
        let alice = agents.iter().find(|a| a.name == "Alice").unwrap();
        assert_eq!(alice.organization.supervises.len(), 1);
    }

    #[tokio::test]
    async fn test_dry_run_migration() {
        let store = Arc::new(InMemoryAgentStore::new());
        let config = MigrationConfig {
            dry_run: true,
            ..Default::default()
        };
        let runner = MigrationRunner::new(store.clone(), config);

        let roles = vec![create_test_role("engineer", "Software Engineer")];
        let report = runner.migrate_roles(&roles).await;

        assert_eq!(report.successful, 1);

        // Verify nothing was stored (dry run)
        let archetypes = store.list_archetypes().await.unwrap();
        assert_eq!(archetypes.len(), 0);
    }
}
