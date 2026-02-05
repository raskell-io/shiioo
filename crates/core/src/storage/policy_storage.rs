//! Persistent policy storage using Redb.
//!
//! Stores archetypes, team policies, organization policies, and policy sets
//! for the agent policy engine.

use anyhow::{Context, Result};
use async_trait::async_trait;
use redb::{Database, ReadableTable, TableDefinition};
use std::path::Path;
use std::sync::Arc;

use crate::agent::{
    policy_engine::PolicyStorage, AgentPolicies, Archetype, ArchetypeId,
};
use crate::types::{OrgId, TeamId};

// Redb table definitions for policy storage
const ARCHETYPES_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("policy_archetypes");
const TEAM_POLICIES_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("team_policies");
const ORG_POLICIES_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("org_policies");
const POLICY_SETS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("policy_sets");

/// Redb-backed policy storage for production use.
///
/// Provides persistent storage for:
/// - Archetypes (agent templates with default policies and budgets)
/// - Team policies (shared policies for all agents in a team)
/// - Organization policies (shared policies for all agents in an org)
/// - Policy sets (reusable policy configurations)
#[derive(Clone)]
pub struct RedbPolicyStorage {
    db: Arc<Database>,
}

impl RedbPolicyStorage {
    /// Create a new policy storage at the given path.
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let db = Database::create(path.as_ref())
            .context("Failed to create policy storage database")?;

        // Create tables if they don't exist
        let write_txn = db.begin_write()?;
        {
            let _ = write_txn.open_table(ARCHETYPES_TABLE)?;
            let _ = write_txn.open_table(TEAM_POLICIES_TABLE)?;
            let _ = write_txn.open_table(ORG_POLICIES_TABLE)?;
            let _ = write_txn.open_table(POLICY_SETS_TABLE)?;
        }
        write_txn.commit()?;

        Ok(Self { db: Arc::new(db) })
    }

    /// Store an archetype.
    pub fn store_archetype(&self, archetype: &Archetype) -> Result<()> {
        let data = serde_json::to_vec(archetype)
            .context("Failed to serialize archetype")?;

        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(ARCHETYPES_TABLE)?;
            table.insert(archetype.id.0.as_str(), data.as_slice())?;
        }
        write_txn.commit()?;

        Ok(())
    }

    /// Delete an archetype.
    pub fn delete_archetype(&self, id: &ArchetypeId) -> Result<bool> {
        let write_txn = self.db.begin_write()?;
        let removed = {
            let mut table = write_txn.open_table(ARCHETYPES_TABLE)?;
            let result = table.remove(id.0.as_str())?;
            result.is_some()
        };
        write_txn.commit()?;
        Ok(removed)
    }

    /// List all archetypes.
    pub fn list_archetypes(&self) -> Result<Vec<Archetype>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(ARCHETYPES_TABLE)?;

        let mut archetypes = Vec::new();
        for entry in table.iter()? {
            let (_, value) = entry?;
            let archetype: Archetype = serde_json::from_slice(value.value())
                .context("Failed to deserialize archetype")?;
            archetypes.push(archetype);
        }

        archetypes.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(archetypes)
    }

    /// Store team policies.
    pub fn store_team_policies(&self, team_id: &TeamId, policies: &AgentPolicies) -> Result<()> {
        let data = serde_json::to_vec(policies)
            .context("Failed to serialize team policies")?;

        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(TEAM_POLICIES_TABLE)?;
            table.insert(team_id.0.as_str(), data.as_slice())?;
        }
        write_txn.commit()?;

        Ok(())
    }

    /// Delete team policies.
    pub fn delete_team_policies(&self, team_id: &TeamId) -> Result<bool> {
        let write_txn = self.db.begin_write()?;
        let removed = {
            let mut table = write_txn.open_table(TEAM_POLICIES_TABLE)?;
            let result = table.remove(team_id.0.as_str())?;
            result.is_some()
        };
        write_txn.commit()?;
        Ok(removed)
    }

    /// Store organization policies.
    pub fn store_org_policies(&self, org_id: &OrgId, policies: &AgentPolicies) -> Result<()> {
        let data = serde_json::to_vec(policies)
            .context("Failed to serialize org policies")?;

        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(ORG_POLICIES_TABLE)?;
            table.insert(org_id.0.as_str(), data.as_slice())?;
        }
        write_txn.commit()?;

        Ok(())
    }

    /// Delete organization policies.
    pub fn delete_org_policies(&self, org_id: &OrgId) -> Result<bool> {
        let write_txn = self.db.begin_write()?;
        let removed = {
            let mut table = write_txn.open_table(ORG_POLICIES_TABLE)?;
            let result = table.remove(org_id.0.as_str())?;
            result.is_some()
        };
        write_txn.commit()?;
        Ok(removed)
    }

    /// Store a policy set.
    pub fn store_policy_set(&self, id: &str, policies: &AgentPolicies) -> Result<()> {
        let data = serde_json::to_vec(policies)
            .context("Failed to serialize policy set")?;

        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(POLICY_SETS_TABLE)?;
            table.insert(id, data.as_slice())?;
        }
        write_txn.commit()?;

        Ok(())
    }

    /// Delete a policy set.
    pub fn delete_policy_set(&self, id: &str) -> Result<bool> {
        let write_txn = self.db.begin_write()?;
        let removed = {
            let mut table = write_txn.open_table(POLICY_SETS_TABLE)?;
            let result = table.remove(id)?;
            result.is_some()
        };
        write_txn.commit()?;
        Ok(removed)
    }

    /// List all policy sets.
    pub fn list_policy_sets(&self) -> Result<Vec<(String, AgentPolicies)>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(POLICY_SETS_TABLE)?;

        let mut policy_sets = Vec::new();
        for entry in table.iter()? {
            let (key, value) = entry?;
            let policies: AgentPolicies = serde_json::from_slice(value.value())
                .context("Failed to deserialize policy set")?;
            policy_sets.push((key.value().to_string(), policies));
        }

        policy_sets.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(policy_sets)
    }
}

#[async_trait]
impl PolicyStorage for RedbPolicyStorage {
    async fn get_archetype(&self, id: &ArchetypeId) -> Result<Option<Archetype>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(ARCHETYPES_TABLE)?;

        match table.get(id.0.as_str())? {
            Some(data) => {
                let archetype = serde_json::from_slice(data.value())
                    .context("Failed to deserialize archetype")?;
                Ok(Some(archetype))
            }
            None => Ok(None),
        }
    }

    async fn get_team_policies(&self, team_id: &TeamId) -> Result<Option<AgentPolicies>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(TEAM_POLICIES_TABLE)?;

        match table.get(team_id.0.as_str())? {
            Some(data) => {
                let policies = serde_json::from_slice(data.value())
                    .context("Failed to deserialize team policies")?;
                Ok(Some(policies))
            }
            None => Ok(None),
        }
    }

    async fn get_org_policies(&self, org_id: &OrgId) -> Result<Option<AgentPolicies>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(ORG_POLICIES_TABLE)?;

        match table.get(org_id.0.as_str())? {
            Some(data) => {
                let policies = serde_json::from_slice(data.value())
                    .context("Failed to deserialize org policies")?;
                Ok(Some(policies))
            }
            None => Ok(None),
        }
    }

    async fn get_policy_set(&self, policy_set_id: &str) -> Result<Option<AgentPolicies>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(POLICY_SETS_TABLE)?;

        match table.get(policy_set_id)? {
            Some(data) => {
                let policies = serde_json::from_slice(data.value())
                    .context("Failed to deserialize policy set")?;
                Ok(Some(policies))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentBudgets, PolicyRule, PolicyScope};
    use tempfile::tempdir;

    fn create_test_archetype(id: &str, name: &str) -> Archetype {
        Archetype {
            id: ArchetypeId::new(id),
            name: name.to_string(),
            description: format!("Test archetype: {}", name),
            skills: Default::default(),
            mcp_bindings: Vec::new(),
            policies: AgentPolicies::default(),
            budgets: AgentBudgets::default(),
            parent: None,
            created_by: "test".to_string(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_store_and_get_archetype() {
        let dir = tempdir().unwrap();
        let storage = RedbPolicyStorage::new(dir.path().join("policies.redb")).unwrap();

        let archetype = create_test_archetype("arch-1", "Test Archetype");
        storage.store_archetype(&archetype).unwrap();

        // Use tokio runtime for async test
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(storage.get_archetype(&ArchetypeId::new("arch-1"))).unwrap();

        assert!(result.is_some());
        let retrieved = result.unwrap();
        assert_eq!(retrieved.id.0, "arch-1");
        assert_eq!(retrieved.name, "Test Archetype");
    }

    #[test]
    fn test_store_and_get_team_policies() {
        let dir = tempdir().unwrap();
        let storage = RedbPolicyStorage::new(dir.path().join("policies.redb")).unwrap();

        let team_id = TeamId::new("team-1");
        let policies = AgentPolicies::new()
            .with_allow(PolicyRule::new("allow-read", "Allow read", PolicyScope::All));

        storage.store_team_policies(&team_id, &policies).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(storage.get_team_policies(&team_id)).unwrap();

        assert!(result.is_some());
        let retrieved = result.unwrap();
        assert_eq!(retrieved.allow.len(), 1);
    }

    #[test]
    fn test_store_and_get_org_policies() {
        let dir = tempdir().unwrap();
        let storage = RedbPolicyStorage::new(dir.path().join("policies.redb")).unwrap();

        let org_id = OrgId::new("org-1");
        let policies = AgentPolicies::new()
            .with_deny(PolicyRule::new("deny-secrets", "Deny secrets", PolicyScope::All));

        storage.store_org_policies(&org_id, &policies).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(storage.get_org_policies(&org_id)).unwrap();

        assert!(result.is_some());
        let retrieved = result.unwrap();
        assert_eq!(retrieved.deny.len(), 1);
    }

    #[test]
    fn test_store_and_get_policy_set() {
        let dir = tempdir().unwrap();
        let storage = RedbPolicyStorage::new(dir.path().join("policies.redb")).unwrap();

        let policies = AgentPolicies::new()
            .with_allow(PolicyRule::new("allow-all", "Allow all", PolicyScope::All));

        storage.store_policy_set("default-policy", &policies).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(storage.get_policy_set("default-policy")).unwrap();

        assert!(result.is_some());
    }

    #[test]
    fn test_list_archetypes() {
        let dir = tempdir().unwrap();
        let storage = RedbPolicyStorage::new(dir.path().join("policies.redb")).unwrap();

        storage.store_archetype(&create_test_archetype("arch-2", "Beta")).unwrap();
        storage.store_archetype(&create_test_archetype("arch-1", "Alpha")).unwrap();

        let archetypes = storage.list_archetypes().unwrap();
        assert_eq!(archetypes.len(), 2);
        assert_eq!(archetypes[0].name, "Alpha"); // Sorted by name
        assert_eq!(archetypes[1].name, "Beta");
    }

    #[test]
    fn test_delete_archetype() {
        let dir = tempdir().unwrap();
        let storage = RedbPolicyStorage::new(dir.path().join("policies.redb")).unwrap();

        let archetype = create_test_archetype("arch-1", "Test");
        storage.store_archetype(&archetype).unwrap();

        let removed = storage.delete_archetype(&ArchetypeId::new("arch-1")).unwrap();
        assert!(removed);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(storage.get_archetype(&ArchetypeId::new("arch-1"))).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_get_nonexistent_returns_none() {
        let dir = tempdir().unwrap();
        let storage = RedbPolicyStorage::new(dir.path().join("policies.redb")).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();

        let archetype = rt.block_on(storage.get_archetype(&ArchetypeId::new("nonexistent"))).unwrap();
        assert!(archetype.is_none());

        let team_policies = rt.block_on(storage.get_team_policies(&TeamId::new("nonexistent"))).unwrap();
        assert!(team_policies.is_none());

        let org_policies = rt.block_on(storage.get_org_policies(&OrgId::new("nonexistent"))).unwrap();
        assert!(org_policies.is_none());

        let policy_set = rt.block_on(storage.get_policy_set("nonexistent")).unwrap();
        assert!(policy_set.is_none());
    }
}
