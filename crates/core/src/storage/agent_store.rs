//! Agent and Archetype storage.
//!
//! Provides persistent storage for agents and archetypes using Redb.

use anyhow::{Context, Result};
use async_trait::async_trait;
use redb::{Database, ReadableTable, TableDefinition};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::agent::{Agent, AgentId, AgentStatus, Archetype, ArchetypeId};

// Redb table definitions
const AGENTS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("agents");
const ARCHETYPES_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("archetypes");

/// Agent storage trait.
#[async_trait]
pub trait AgentStore: Send + Sync {
    /// Store an agent.
    async fn store_agent(&self, agent: &Agent) -> Result<()>;

    /// Get an agent by ID.
    async fn get_agent(&self, id: &AgentId) -> Result<Option<Agent>>;

    /// List all agents.
    async fn list_agents(&self) -> Result<Vec<Agent>>;

    /// List agents by status.
    async fn list_agents_by_status(&self, status: AgentStatus) -> Result<Vec<Agent>>;

    /// List agents by team.
    async fn list_agents_by_team(&self, team_id: &str) -> Result<Vec<Agent>>;

    /// Delete an agent.
    async fn delete_agent(&self, id: &AgentId) -> Result<bool>;

    /// Update agent status.
    async fn update_agent_status(&self, id: &AgentId, status: AgentStatus) -> Result<()>;

    /// Store an archetype.
    async fn store_archetype(&self, archetype: &Archetype) -> Result<()>;

    /// Get an archetype by ID.
    async fn get_archetype(&self, id: &ArchetypeId) -> Result<Option<Archetype>>;

    /// List all archetypes.
    async fn list_archetypes(&self) -> Result<Vec<Archetype>>;

    /// Delete an archetype.
    async fn delete_archetype(&self, id: &ArchetypeId) -> Result<bool>;
}

/// In-memory agent store for testing.
#[derive(Debug, Default)]
pub struct InMemoryAgentStore {
    agents: RwLock<HashMap<String, Agent>>,
    archetypes: RwLock<HashMap<String, Archetype>>,
}

impl InMemoryAgentStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl AgentStore for InMemoryAgentStore {
    async fn store_agent(&self, agent: &Agent) -> Result<()> {
        let mut agents = self.agents.write().await;
        agents.insert(agent.id.0.clone(), agent.clone());
        Ok(())
    }

    async fn get_agent(&self, id: &AgentId) -> Result<Option<Agent>> {
        let agents = self.agents.read().await;
        Ok(agents.get(&id.0).cloned())
    }

    async fn list_agents(&self) -> Result<Vec<Agent>> {
        let agents = self.agents.read().await;
        let mut list: Vec<_> = agents.values().cloned().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(list)
    }

    async fn list_agents_by_status(&self, status: AgentStatus) -> Result<Vec<Agent>> {
        let agents = self.agents.read().await;
        let list: Vec<_> = agents
            .values()
            .filter(|a| a.status == status)
            .cloned()
            .collect();
        Ok(list)
    }

    async fn list_agents_by_team(&self, team_id: &str) -> Result<Vec<Agent>> {
        let agents = self.agents.read().await;
        let list: Vec<_> = agents
            .values()
            .filter(|a| a.organization.team.0 == team_id)
            .cloned()
            .collect();
        Ok(list)
    }

    async fn delete_agent(&self, id: &AgentId) -> Result<bool> {
        let mut agents = self.agents.write().await;
        Ok(agents.remove(&id.0).is_some())
    }

    async fn update_agent_status(&self, id: &AgentId, status: AgentStatus) -> Result<()> {
        let mut agents = self.agents.write().await;
        if let Some(agent) = agents.get_mut(&id.0) {
            agent.status = status;
            agent.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn store_archetype(&self, archetype: &Archetype) -> Result<()> {
        let mut archetypes = self.archetypes.write().await;
        archetypes.insert(archetype.id.0.clone(), archetype.clone());
        Ok(())
    }

    async fn get_archetype(&self, id: &ArchetypeId) -> Result<Option<Archetype>> {
        let archetypes = self.archetypes.read().await;
        Ok(archetypes.get(&id.0).cloned())
    }

    async fn list_archetypes(&self) -> Result<Vec<Archetype>> {
        let archetypes = self.archetypes.read().await;
        let mut list: Vec<_> = archetypes.values().cloned().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(list)
    }

    async fn delete_archetype(&self, id: &ArchetypeId) -> Result<bool> {
        let mut archetypes = self.archetypes.write().await;
        Ok(archetypes.remove(&id.0).is_some())
    }
}

/// Redb-backed agent store for production.
#[derive(Clone)]
pub struct RedbAgentStore {
    db: Arc<Database>,
}

impl RedbAgentStore {
    /// Create a new Redb agent store.
    pub fn new(path: PathBuf) -> Result<Self> {
        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("Failed to create agent store directory")?;
        }

        let db = Database::create(&path).context("Failed to create agent store database")?;

        // Initialize tables
        let write_txn = db.begin_write().context("Failed to begin write transaction")?;
        {
            let _agents_table = write_txn
                .open_table(AGENTS_TABLE)
                .context("Failed to open agents table")?;
            let _archetypes_table = write_txn
                .open_table(ARCHETYPES_TABLE)
                .context("Failed to open archetypes table")?;
        }
        write_txn.commit().context("Failed to commit transaction")?;

        Ok(Self { db: Arc::new(db) })
    }

    /// Open an existing Redb agent store from a shared database.
    pub fn from_db(db: Arc<Database>) -> Result<Self> {
        // Initialize tables if needed
        let write_txn = db.begin_write().context("Failed to begin write transaction")?;
        {
            let _agents_table = write_txn
                .open_table(AGENTS_TABLE)
                .context("Failed to open agents table")?;
            let _archetypes_table = write_txn
                .open_table(ARCHETYPES_TABLE)
                .context("Failed to open archetypes table")?;
        }
        write_txn.commit().context("Failed to commit transaction")?;

        Ok(Self { db })
    }
}

#[async_trait]
impl AgentStore for RedbAgentStore {
    async fn store_agent(&self, agent: &Agent) -> Result<()> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        {
            let mut table = write_txn
                .open_table(AGENTS_TABLE)
                .context("Failed to open agents table")?;

            let key = &agent.id.0;
            let value = serde_json::to_vec(agent).context("Failed to serialize agent")?;

            table
                .insert(key.as_str(), value.as_slice())
                .context("Failed to insert agent")?;
        }
        write_txn.commit().context("Failed to commit")?;
        Ok(())
    }

    async fn get_agent(&self, id: &AgentId) -> Result<Option<Agent>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn
            .open_table(AGENTS_TABLE)
            .context("Failed to open agents table")?;

        let value = table.get(id.0.as_str()).context("Failed to get agent")?;

        match value {
            Some(guard) => {
                let bytes = guard.value();
                let agent: Agent =
                    serde_json::from_slice(bytes).context("Failed to deserialize agent")?;
                Ok(Some(agent))
            }
            None => Ok(None),
        }
    }

    async fn list_agents(&self) -> Result<Vec<Agent>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn
            .open_table(AGENTS_TABLE)
            .context("Failed to open agents table")?;

        let mut agents = Vec::new();
        for item in table.iter().context("Failed to iterate agents")? {
            let (_key, value) = item.context("Failed to read item")?;
            let agent: Agent =
                serde_json::from_slice(value.value()).context("Failed to deserialize agent")?;
            agents.push(agent);
        }

        agents.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(agents)
    }

    async fn list_agents_by_status(&self, status: AgentStatus) -> Result<Vec<Agent>> {
        let all_agents = self.list_agents().await?;
        Ok(all_agents.into_iter().filter(|a| a.status == status).collect())
    }

    async fn list_agents_by_team(&self, team_id: &str) -> Result<Vec<Agent>> {
        let all_agents = self.list_agents().await?;
        Ok(all_agents
            .into_iter()
            .filter(|a| a.organization.team.0 == team_id)
            .collect())
    }

    async fn delete_agent(&self, id: &AgentId) -> Result<bool> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        let removed = {
            let mut table = write_txn
                .open_table(AGENTS_TABLE)
                .context("Failed to open agents table")?;

            let result = table
                .remove(id.0.as_str())
                .context("Failed to delete agent")?;
            result.is_some()
        };
        write_txn.commit().context("Failed to commit")?;
        Ok(removed)
    }

    async fn update_agent_status(&self, id: &AgentId, status: AgentStatus) -> Result<()> {
        let mut agent = self
            .get_agent(id)
            .await?
            .context("Employee not found")?;

        agent.status = status;
        agent.updated_at = chrono::Utc::now();

        self.store_agent(&agent).await
    }

    async fn store_archetype(&self, archetype: &Archetype) -> Result<()> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        {
            let mut table = write_txn
                .open_table(ARCHETYPES_TABLE)
                .context("Failed to open archetypes table")?;

            let key = &archetype.id.0;
            let value = serde_json::to_vec(archetype).context("Failed to serialize archetype")?;

            table
                .insert(key.as_str(), value.as_slice())
                .context("Failed to insert archetype")?;
        }
        write_txn.commit().context("Failed to commit")?;
        Ok(())
    }

    async fn get_archetype(&self, id: &ArchetypeId) -> Result<Option<Archetype>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn
            .open_table(ARCHETYPES_TABLE)
            .context("Failed to open archetypes table")?;

        let value = table.get(id.0.as_str()).context("Failed to get archetype")?;

        match value {
            Some(guard) => {
                let bytes = guard.value();
                let archetype: Archetype =
                    serde_json::from_slice(bytes).context("Failed to deserialize archetype")?;
                Ok(Some(archetype))
            }
            None => Ok(None),
        }
    }

    async fn list_archetypes(&self) -> Result<Vec<Archetype>> {
        let read_txn = self.db.begin_read().context("Failed to begin read")?;
        let table = read_txn
            .open_table(ARCHETYPES_TABLE)
            .context("Failed to open archetypes table")?;

        let mut archetypes = Vec::new();
        for item in table.iter().context("Failed to iterate archetypes")? {
            let (_key, value) = item.context("Failed to read item")?;
            let archetype: Archetype =
                serde_json::from_slice(value.value()).context("Failed to deserialize archetype")?;
            archetypes.push(archetype);
        }

        archetypes.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(archetypes)
    }

    async fn delete_archetype(&self, id: &ArchetypeId) -> Result<bool> {
        let write_txn = self.db.begin_write().context("Failed to begin write")?;
        let removed = {
            let mut table = write_txn
                .open_table(ARCHETYPES_TABLE)
                .context("Failed to open archetypes table")?;

            let result = table
                .remove(id.0.as_str())
                .context("Failed to delete archetype")?;
            result.is_some()
        };
        write_txn.commit().context("Failed to commit")?;
        Ok(removed)
    }
}

/// Agent events for the event log.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// Agent was created.
    Created {
        agent_id: String,
        name: String,
        archetype_id: Option<String>,
        team_id: String,
    },
    /// Agent status changed.
    StatusChanged {
        agent_id: String,
        old_status: AgentStatus,
        new_status: AgentStatus,
    },
    /// Agent's skills were updated.
    SkillsUpdated {
        agent_id: String,
        activated_skills: Vec<String>,
        deactivated_skills: Vec<String>,
    },
    /// Agent's policies were updated.
    PoliciesUpdated {
        agent_id: String,
        added_rules: Vec<String>,
        removed_rules: Vec<String>,
    },
    /// Agent's MCP bindings were updated.
    McpBindingsUpdated {
        agent_id: String,
        added_bindings: Vec<String>,
        removed_bindings: Vec<String>,
    },
    /// Agent was archived (soft delete).
    Archived {
        agent_id: String,
        reason: Option<String>,
    },
    /// Agent executed an action.
    ActionExecuted {
        agent_id: String,
        action_type: String,
        tool_id: Option<String>,
        resource: Option<String>,
        decision: String,
    },
    /// Agent delegated a task.
    TaskDelegated {
        agent_id: String,
        target_agent_id: String,
        task_description: String,
    },
    /// Agent escalated a task.
    TaskEscalated {
        agent_id: String,
        escalation_target: String,
        reason: String,
    },
}

impl AgentEvent {
    /// Get the agent ID for this event.
    pub fn agent_id(&self) -> &str {
        match self {
            Self::Created { agent_id, .. } => agent_id,
            Self::StatusChanged { agent_id, .. } => agent_id,
            Self::SkillsUpdated { agent_id, .. } => agent_id,
            Self::PoliciesUpdated { agent_id, .. } => agent_id,
            Self::McpBindingsUpdated { agent_id, .. } => agent_id,
            Self::Archived { agent_id, .. } => agent_id,
            Self::ActionExecuted { agent_id, .. } => agent_id,
            Self::TaskDelegated { agent_id, .. } => agent_id,
            Self::TaskEscalated { agent_id, .. } => agent_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentBudgets, AgentOrganization, AgentPolicies, AgentSkills};
    use crate::types::TeamId;

    fn create_test_agent(id: &str, name: &str) -> Agent {
        Agent::builder(AgentId::new(id), name)
            .description("Test agent")
            .organization(AgentOrganization {
                team: TeamId::new("test-team"),
                reports_to: None,
                supervises: vec![],
                peers: vec![],
            })
            .build()
    }

    #[tokio::test]
    async fn test_in_memory_store_agent_crud() {
        let store = InMemoryAgentStore::new();

        // Create
        let agent = create_test_agent("agent-1", "Agent One");
        store.store_agent(&agent).await.unwrap();

        // Read
        let retrieved = store.get_agent(&AgentId::new("agent-1")).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "Agent One");

        // List
        let list = store.list_agents().await.unwrap();
        assert_eq!(list.len(), 1);

        // Delete
        let deleted = store.delete_agent(&AgentId::new("agent-1")).await.unwrap();
        assert!(deleted);

        // Verify deleted
        let retrieved = store.get_agent(&AgentId::new("agent-1")).await.unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_in_memory_store_agent_status_update() {
        let store = InMemoryAgentStore::new();

        let agent = create_test_agent("agent-1", "Agent One");
        store.store_agent(&agent).await.unwrap();

        // Update status
        store
            .update_agent_status(&AgentId::new("agent-1"), AgentStatus::Suspended)
            .await
            .unwrap();

        let retrieved = store.get_agent(&AgentId::new("agent-1")).await.unwrap().unwrap();
        assert_eq!(retrieved.status, AgentStatus::Suspended);
    }

    #[tokio::test]
    async fn test_in_memory_store_list_by_status() {
        let store = InMemoryAgentStore::new();

        let agent1 = create_test_agent("agent-1", "Agent One");
        let mut agent2 = create_test_agent("agent-2", "Agent Two");
        agent2.status = AgentStatus::Paused;

        store.store_agent(&agent1).await.unwrap();
        store.store_agent(&agent2).await.unwrap();

        let active = store.list_agents_by_status(AgentStatus::Active).await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id.0, "agent-1");

        let paused = store.list_agents_by_status(AgentStatus::Paused).await.unwrap();
        assert_eq!(paused.len(), 1);
        assert_eq!(paused[0].id.0, "agent-2");
    }

    #[tokio::test]
    async fn test_in_memory_store_archetype_crud() {
        let store = InMemoryAgentStore::new();

        let archetype = crate::agent::common::software_engineer();
        store.store_archetype(&archetype).await.unwrap();

        let retrieved = store
            .get_archetype(&ArchetypeId::new("software-engineer"))
            .await
            .unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "Software Engineer");

        let list = store.list_archetypes().await.unwrap();
        assert_eq!(list.len(), 1);

        let deleted = store
            .delete_archetype(&ArchetypeId::new("software-engineer"))
            .await
            .unwrap();
        assert!(deleted);
    }

    #[tokio::test]
    async fn test_redb_store_agent_crud() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("agents.redb");

        let store = RedbAgentStore::new(db_path).unwrap();

        // Create
        let agent = create_test_agent("agent-1", "Agent One");
        store.store_agent(&agent).await.unwrap();

        // Read
        let retrieved = store.get_agent(&AgentId::new("agent-1")).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "Agent One");

        // List
        let list = store.list_agents().await.unwrap();
        assert_eq!(list.len(), 1);

        // Delete
        let deleted = store.delete_agent(&AgentId::new("agent-1")).await.unwrap();
        assert!(deleted);

        // Verify deleted
        let retrieved = store.get_agent(&AgentId::new("agent-1")).await.unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_redb_store_archetype_crud() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("agents.redb");

        let store = RedbAgentStore::new(db_path).unwrap();

        let archetype = crate::agent::common::software_engineer();
        store.store_archetype(&archetype).await.unwrap();

        let retrieved = store
            .get_archetype(&ArchetypeId::new("software-engineer"))
            .await
            .unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "Software Engineer");
    }

    #[test]
    fn test_agent_event_serialization() {
        let event = AgentEvent::Created {
            agent_id: "agent-1".to_string(),
            name: "Agent One".to_string(),
            archetype_id: Some("software-engineer".to_string()),
            team_id: "platform-team".to_string(),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"created\""));

        let deserialized: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.agent_id(), "agent-1");
    }
}
