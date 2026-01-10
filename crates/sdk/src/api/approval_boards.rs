//! Approval Boards API endpoints.

use crate::client::ShiiooClient;
use crate::error::ShiiooResult;
use serde::{Deserialize, Serialize};
use shiioo_core::types::{ApprovalBoard, ApprovalBoardId};

/// Approval Boards API for managing approval boards.
pub struct ApprovalBoardsApi<'a> {
    client: &'a ShiiooClient,
}

impl<'a> ApprovalBoardsApi<'a> {
    pub(crate) fn new(client: &'a ShiiooClient) -> Self {
        Self { client }
    }

    /// List all approval boards.
    pub async fn list(&self) -> ShiiooResult<Vec<ApprovalBoard>> {
        let response: ListApprovalBoardsResponse =
            self.client.http.get("/api/approval-boards").await?;
        Ok(response.boards)
    }

    /// Get a specific approval board.
    pub async fn get(&self, board_id: &ApprovalBoardId) -> ShiiooResult<ApprovalBoard> {
        self.client
            .http
            .get(&format!("/api/approval-boards/{}", board_id.0))
            .await
    }

    /// Create an approval board.
    pub async fn create(&self, board: &ApprovalBoard) -> ShiiooResult<CreateApprovalBoardResponse> {
        self.client.http.post("/api/approval-boards", board).await
    }

    /// Delete an approval board.
    pub async fn delete(
        &self,
        board_id: &ApprovalBoardId,
    ) -> ShiiooResult<DeleteApprovalBoardResponse> {
        self.client
            .http
            .delete(&format!("/api/approval-boards/{}", board_id.0))
            .await
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ListApprovalBoardsResponse {
    boards: Vec<ApprovalBoard>,
}

/// Response from creating an approval board.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateApprovalBoardResponse {
    pub board_id: String,
    pub message: String,
}

/// Response from deleting an approval board.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteApprovalBoardResponse {
    pub message: String,
}
