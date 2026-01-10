//! Cluster API endpoints.

use crate::client::ShiiooClient;
use crate::error::ShiiooResult;
use serde::{Deserialize, Serialize};
use shiioo_core::cluster::{ClusterNode, NodeId};
use std::collections::HashMap;

/// Cluster API for managing cluster nodes.
pub struct ClusterApi<'a> {
    client: &'a ShiiooClient,
}

impl<'a> ClusterApi<'a> {
    pub(crate) fn new(client: &'a ShiiooClient) -> Self {
        Self { client }
    }

    /// List all cluster nodes.
    pub async fn nodes(&self) -> ShiiooResult<Vec<ClusterNode>> {
        let response: ListNodesResponse = self.client.http.get("/api/cluster/nodes").await?;
        Ok(response.nodes)
    }

    /// Get a specific cluster node.
    pub async fn get_node(&self, node_id: &NodeId) -> ShiiooResult<ClusterNode> {
        self.client
            .http
            .get(&format!("/api/cluster/nodes/{}", node_id.0))
            .await
    }

    /// Register a new cluster node.
    pub async fn register_node(&self, request: RegisterNodeRequest) -> ShiiooResult<ClusterNode> {
        self.client.http.post("/api/cluster/nodes", &request).await
    }

    /// Remove a cluster node.
    pub async fn remove_node(&self, node_id: &NodeId) -> ShiiooResult<RemoveNodeResponse> {
        self.client
            .http
            .delete(&format!("/api/cluster/nodes/{}", node_id.0))
            .await
    }

    /// Send a heartbeat for a node.
    pub async fn heartbeat(&self, node_id: &NodeId) -> ShiiooResult<HeartbeatResponse> {
        self.client
            .http
            .post(&format!("/api/cluster/nodes/{}/heartbeat", node_id.0), &())
            .await
    }

    /// Get the current leader node.
    pub async fn leader(&self) -> ShiiooResult<Option<ClusterNode>> {
        let response: LeaderResponse = self.client.http.get("/api/cluster/leader").await?;
        Ok(response.leader)
    }

    /// Get cluster health status.
    pub async fn health(&self) -> ShiiooResult<ClusterHealthResponse> {
        self.client.http.get("/api/cluster/health").await
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ListNodesResponse {
    nodes: Vec<ClusterNode>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LeaderResponse {
    leader: Option<ClusterNode>,
}

/// Request to register a cluster node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterNodeRequest {
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
}

/// Response from removing a node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveNodeResponse {
    pub message: String,
}

/// Response from a heartbeat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatResponse {
    pub message: String,
}

/// Cluster health status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterHealthResponse {
    pub total_nodes: usize,
    pub healthy_nodes: usize,
    pub has_leader: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leader_id: Option<String>,
}
