use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Unique identifier for a cluster node
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub String);

impl NodeId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Generate a new random node ID
    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

/// Cluster node information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterNode {
    pub id: NodeId,
    pub address: String,
    pub region: Option<String>,
    pub status: NodeStatus,
    pub role: NodeRole,
    pub last_heartbeat: DateTime<Utc>,
    pub started_at: DateTime<Utc>,
    pub metadata: HashMap<String, String>,
}

/// Node status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeStatus {
    Healthy,
    Degraded,
    Unhealthy,
    Offline,
}

/// Node role in the cluster
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeRole {
    Leader,
    Follower,
    Candidate,
}

/// Cluster manager for node discovery and health tracking
pub struct ClusterManager {
    local_node_id: NodeId,
    nodes: Arc<Mutex<HashMap<NodeId, ClusterNode>>>,
    heartbeat_timeout_secs: i64,
}

impl ClusterManager {
    /// Create a new cluster manager
    pub fn new(local_node_id: NodeId, heartbeat_timeout_secs: i64) -> Self {
        Self {
            local_node_id,
            nodes: Arc::new(Mutex::new(HashMap::new())),
            heartbeat_timeout_secs,
        }
    }

    /// Register a node in the cluster
    pub fn register_node(&self, node: ClusterNode) -> anyhow::Result<()> {
        let mut nodes = self.nodes.lock().unwrap();
        nodes.insert(node.id.clone(), node);
        Ok(())
    }

    /// Update node heartbeat
    pub fn heartbeat(&self, node_id: &NodeId) -> anyhow::Result<()> {
        let mut nodes = self.nodes.lock().unwrap();
        let node = nodes
            .get_mut(node_id)
            .ok_or_else(|| anyhow::anyhow!("Node not found: {}", node_id.0))?;

        node.last_heartbeat = Utc::now();
        node.status = NodeStatus::Healthy;

        Ok(())
    }

    /// Get a node by ID
    pub fn get_node(&self, node_id: &NodeId) -> Option<ClusterNode> {
        self.nodes.lock().unwrap().get(node_id).cloned()
    }

    /// List all nodes
    pub fn list_nodes(&self) -> Vec<ClusterNode> {
        self.nodes.lock().unwrap().values().cloned().collect()
    }

    /// List healthy nodes
    pub fn list_healthy_nodes(&self) -> Vec<ClusterNode> {
        self.nodes
            .lock()
            .unwrap()
            .values()
            .filter(|n| n.status == NodeStatus::Healthy)
            .cloned()
            .collect()
    }

    /// Get the current leader node
    pub fn get_leader(&self) -> Option<ClusterNode> {
        self.nodes
            .lock()
            .unwrap()
            .values()
            .find(|n| n.role == NodeRole::Leader)
            .cloned()
    }

    /// Check if local node is the leader
    pub fn is_leader(&self) -> bool {
        self.get_leader()
            .map(|leader| leader.id == self.local_node_id)
            .unwrap_or(false)
    }

    /// Remove a node from the cluster
    pub fn remove_node(&self, node_id: &NodeId) -> anyhow::Result<()> {
        let mut nodes = self.nodes.lock().unwrap();
        nodes
            .remove(node_id)
            .ok_or_else(|| anyhow::anyhow!("Node not found: {}", node_id.0))?;
        Ok(())
    }

    /// Check for stale nodes and mark them as unhealthy
    pub fn check_stale_nodes(&self) -> Vec<NodeId> {
        let mut nodes = self.nodes.lock().unwrap();
        let now = Utc::now();
        let timeout = Duration::seconds(self.heartbeat_timeout_secs);
        let mut stale_nodes = Vec::new();

        for (node_id, node) in nodes.iter_mut() {
            if now - node.last_heartbeat > timeout {
                node.status = NodeStatus::Unhealthy;
                stale_nodes.push(node_id.clone());
            }
        }

        stale_nodes
    }

    /// Get cluster size
    pub fn cluster_size(&self) -> usize {
        self.nodes.lock().unwrap().len()
    }

    /// Get healthy node count
    pub fn healthy_node_count(&self) -> usize {
        self.list_healthy_nodes().len()
    }
}

/// Distributed lock for coordinating exclusive access
pub struct DistributedLock {
    locks: Arc<Mutex<HashMap<String, LockInfo>>>,
    default_ttl_secs: i64,
}

#[derive(Debug, Clone)]
struct LockInfo {
    holder: NodeId,
    acquired_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

impl DistributedLock {
    /// Create a new distributed lock
    pub fn new(default_ttl_secs: i64) -> Self {
        Self {
            locks: Arc::new(Mutex::new(HashMap::new())),
            default_ttl_secs,
        }
    }

    /// Acquire a lock
    pub fn acquire(&self, key: &str, holder: NodeId) -> anyhow::Result<bool> {
        self.acquire_with_ttl(key, holder, self.default_ttl_secs)
    }

    /// Acquire a lock with custom TTL
    pub fn acquire_with_ttl(
        &self,
        key: &str,
        holder: NodeId,
        ttl_secs: i64,
    ) -> anyhow::Result<bool> {
        let mut locks = self.locks.lock().unwrap();
        let now = Utc::now();

        // Check if lock exists and is still valid
        if let Some(lock_info) = locks.get(key) {
            if lock_info.expires_at > now {
                // Lock is held by someone else
                return Ok(false);
            }
        }

        // Acquire the lock
        let lock_info = LockInfo {
            holder: holder.clone(),
            acquired_at: now,
            expires_at: now + Duration::seconds(ttl_secs),
        };

        locks.insert(key.to_string(), lock_info);
        Ok(true)
    }

    /// Release a lock
    pub fn release(&self, key: &str, holder: &NodeId) -> anyhow::Result<bool> {
        let mut locks = self.locks.lock().unwrap();

        if let Some(lock_info) = locks.get(key) {
            if &lock_info.holder == holder {
                locks.remove(key);
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Check if a lock is held
    pub fn is_locked(&self, key: &str) -> bool {
        let locks = self.locks.lock().unwrap();
        let now = Utc::now();

        locks
            .get(key)
            .map(|lock_info| lock_info.expires_at > now)
            .unwrap_or(false)
    }

    /// Get lock holder
    pub fn get_holder(&self, key: &str) -> Option<NodeId> {
        let locks = self.locks.lock().unwrap();
        let now = Utc::now();

        locks.get(key).and_then(|lock_info| {
            if lock_info.expires_at > now {
                Some(lock_info.holder.clone())
            } else {
                None
            }
        })
    }

    /// Clean up expired locks
    pub fn cleanup_expired(&self) -> usize {
        let mut locks = self.locks.lock().unwrap();
        let now = Utc::now();

        let expired_keys: Vec<String> = locks
            .iter()
            .filter(|(_, lock_info)| lock_info.expires_at <= now)
            .map(|(key, _)| key.clone())
            .collect();

        let count = expired_keys.len();
        for key in expired_keys {
            locks.remove(&key);
        }

        count
    }
}

/// Leader election coordinator
pub struct LeaderElection {
    cluster_manager: Arc<ClusterManager>,
    lock: Arc<DistributedLock>,
    election_key: String,
    lease_duration_secs: i64,
}

impl LeaderElection {
    /// Create a new leader election coordinator
    pub fn new(
        cluster_manager: Arc<ClusterManager>,
        lock: Arc<DistributedLock>,
        lease_duration_secs: i64,
    ) -> Self {
        Self {
            cluster_manager,
            lock,
            election_key: "leader_election".to_string(),
            lease_duration_secs,
        }
    }

    /// Attempt to become leader
    pub fn try_become_leader(&self, node_id: &NodeId) -> anyhow::Result<bool> {
        // Try to acquire the leader lock
        let acquired = self
            .lock
            .acquire_with_ttl(&self.election_key, node_id.clone(), self.lease_duration_secs)?;

        if acquired {
            // Update node role to leader
            if let Some(mut node) = self.cluster_manager.get_node(node_id) {
                node.role = NodeRole::Leader;
                self.cluster_manager.register_node(node)?;
            }

            tracing::info!("Node {} became leader", node_id.0);
        }

        Ok(acquired)
    }

    /// Renew leadership
    pub fn renew_leadership(&self, node_id: &NodeId) -> anyhow::Result<bool> {
        // Check if we still hold the lock
        if let Some(holder) = self.lock.get_holder(&self.election_key) {
            if &holder == node_id {
                // Renew by re-acquiring
                return self.try_become_leader(node_id);
            }
        }

        Ok(false)
    }

    /// Step down as leader
    pub fn step_down(&self, node_id: &NodeId) -> anyhow::Result<()> {
        // Release the leader lock
        self.lock.release(&self.election_key, node_id)?;

        // Update node role to follower
        if let Some(mut node) = self.cluster_manager.get_node(node_id) {
            node.role = NodeRole::Follower;
            self.cluster_manager.register_node(node)?;
        }

        tracing::info!("Node {} stepped down as leader", node_id.0);
        Ok(())
    }

    /// Get current leader
    pub fn get_current_leader(&self) -> Option<NodeId> {
        self.lock.get_holder(&self.election_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_node(id: &str) -> ClusterNode {
        ClusterNode {
            id: NodeId::new(id),
            address: format!("http://node-{}", id),
            region: Some("us-east-1".to_string()),
            status: NodeStatus::Healthy,
            role: NodeRole::Follower,
            last_heartbeat: Utc::now(),
            started_at: Utc::now(),
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn test_register_node() {
        let manager = ClusterManager::new(NodeId::new("node1"), 30);
        let node = create_test_node("node1");

        manager.register_node(node.clone()).unwrap();

        let retrieved = manager.get_node(&node.id).unwrap();
        assert_eq!(retrieved.id, node.id);
    }

    #[test]
    fn test_heartbeat() {
        let manager = ClusterManager::new(NodeId::new("node1"), 30);
        let node = create_test_node("node1");

        manager.register_node(node.clone()).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(100));

        manager.heartbeat(&node.id).unwrap();

        let retrieved = manager.get_node(&node.id).unwrap();
        assert!(retrieved.last_heartbeat > node.last_heartbeat);
    }

    #[test]
    fn test_list_healthy_nodes() {
        let manager = ClusterManager::new(NodeId::new("node1"), 30);

        let mut node1 = create_test_node("node1");
        let node2 = create_test_node("node2");

        manager.register_node(node1.clone()).unwrap();
        manager.register_node(node2.clone()).unwrap();

        // Mark node1 as unhealthy
        node1.status = NodeStatus::Unhealthy;
        manager.register_node(node1).unwrap();

        let healthy = manager.list_healthy_nodes();
        assert_eq!(healthy.len(), 1);
        assert_eq!(healthy[0].id, node2.id);
    }

    #[test]
    fn test_distributed_lock_acquire() {
        let lock = DistributedLock::new(30);
        let node1 = NodeId::new("node1");
        let node2 = NodeId::new("node2");

        // Node1 acquires lock
        assert!(lock.acquire("test_lock", node1.clone()).unwrap());

        // Node2 cannot acquire same lock
        assert!(!lock.acquire("test_lock", node2.clone()).unwrap());

        // Node1 can release
        assert!(lock.release("test_lock", &node1).unwrap());

        // Now node2 can acquire
        assert!(lock.acquire("test_lock", node2).unwrap());
    }

    #[test]
    fn test_distributed_lock_expiry() {
        let lock = DistributedLock::new(1); // 1 second TTL
        let node1 = NodeId::new("node1");

        assert!(lock.acquire("test_lock", node1.clone()).unwrap());
        assert!(lock.is_locked("test_lock"));

        std::thread::sleep(std::time::Duration::from_secs(2));

        // Lock should have expired
        let cleaned = lock.cleanup_expired();
        assert_eq!(cleaned, 1);
        assert!(!lock.is_locked("test_lock"));
    }

    #[test]
    fn test_leader_election() {
        let cluster_manager = Arc::new(ClusterManager::new(NodeId::new("node1"), 30));
        let lock = Arc::new(DistributedLock::new(30));

        let node1 = create_test_node("node1");
        let node2 = create_test_node("node2");

        cluster_manager.register_node(node1.clone()).unwrap();
        cluster_manager.register_node(node2.clone()).unwrap();

        let election = LeaderElection::new(cluster_manager.clone(), lock, 30);

        // Node1 becomes leader
        assert!(election.try_become_leader(&node1.id).unwrap());
        assert_eq!(election.get_current_leader(), Some(node1.id.clone()));

        // Node2 cannot become leader
        assert!(!election.try_become_leader(&node2.id).unwrap());

        // Node1 steps down
        election.step_down(&node1.id).unwrap();
        assert!(election.get_current_leader().is_none());

        // Now node2 can become leader
        assert!(election.try_become_leader(&node2.id).unwrap());
    }
}
