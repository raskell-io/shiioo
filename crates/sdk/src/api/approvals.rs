//! Approvals API endpoints.

use crate::client::ShiiooClient;
use crate::error::ShiiooResult;
use serde::{Deserialize, Serialize};
use shiioo_core::types::{Approval, ApprovalId, PersonId, VoteDecision};

/// Approvals API for managing approvals.
pub struct ApprovalsApi<'a> {
    client: &'a ShiiooClient,
}

impl<'a> ApprovalsApi<'a> {
    pub(crate) fn new(client: &'a ShiiooClient) -> Self {
        Self { client }
    }

    /// List all approvals.
    pub async fn list(&self) -> ShiiooResult<Vec<Approval>> {
        let response: ListApprovalsResponse = self.client.http.get("/api/approvals").await?;
        Ok(response.approvals)
    }

    /// Get a specific approval.
    pub async fn get(&self, approval_id: &ApprovalId) -> ShiiooResult<Approval> {
        self.client
            .http
            .get(&format!("/api/approvals/{}", approval_id.0))
            .await
    }

    /// Cast a vote on an approval.
    pub async fn vote(
        &self,
        approval_id: &ApprovalId,
        request: CastVoteRequest,
    ) -> ShiiooResult<CastVoteResponse> {
        self.client
            .http
            .post(&format!("/api/approvals/{}/vote", approval_id.0), &request)
            .await
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ListApprovalsResponse {
    approvals: Vec<Approval>,
}

/// Request to cast a vote.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastVoteRequest {
    pub voter_id: PersonId,
    pub decision: VoteDecision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

/// Response from casting a vote.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastVoteResponse {
    pub message: String,
}
