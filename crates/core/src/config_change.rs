use crate::approval::ApprovalManager;
use crate::types::{
    ApprovalBoardId, ApprovalSubject, ConfigChange, ConfigChangeId, ConfigChangeStatus,
    ConfigChangeType,
};
use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Config change manager with approval workflow
pub struct ConfigChangeManager {
    changes: Arc<Mutex<HashMap<ConfigChangeId, ConfigChange>>>,
    approval_manager: Arc<ApprovalManager>,
}

impl ConfigChangeManager {
    /// Create a new config change manager
    pub fn new(approval_manager: Arc<ApprovalManager>) -> Self {
        Self {
            changes: Arc::new(Mutex::new(HashMap::new())),
            approval_manager,
        }
    }

    /// Propose a configuration change
    pub fn propose_change(
        &self,
        change_type: ConfigChangeType,
        description: String,
        before: Option<String>,
        after: String,
        proposed_by: String,
        approval_board: Option<ApprovalBoardId>,
    ) -> Result<ConfigChange> {
        let change_id = ConfigChangeId::new(uuid::Uuid::new_v4().to_string());

        // Create approval if board is specified
        let (approval_id, status) = if let Some(board_id) = approval_board {
            let approval = self.approval_manager.create_approval(
                board_id,
                ApprovalSubject::ConfigChange {
                    change_id: change_id.clone(),
                },
                proposed_by.clone(),
            )?;
            (Some(approval.id), ConfigChangeStatus::PendingApproval)
        } else {
            (None, ConfigChangeStatus::Proposed)
        };

        let change = ConfigChange {
            id: change_id,
            change_type,
            description,
            proposed_by,
            approval_id,
            status,
            before,
            after,
            applied_at: None,
            created_at: Utc::now(),
        };

        self.changes
            .lock()
            .unwrap()
            .insert(change.id.clone(), change.clone());

        tracing::info!(
            "Proposed config change {}: {}",
            change.id.0,
            change.description
        );

        Ok(change)
    }

    /// Get a config change
    pub fn get_change(&self, change_id: &ConfigChangeId) -> Option<ConfigChange> {
        self.changes.lock().unwrap().get(change_id).cloned()
    }

    /// List all config changes
    pub fn list_changes(&self) -> Vec<ConfigChange> {
        self.changes.lock().unwrap().values().cloned().collect()
    }

    /// List changes by status
    pub fn list_changes_by_status(&self, status: ConfigChangeStatus) -> Vec<ConfigChange> {
        self.changes
            .lock()
            .unwrap()
            .values()
            .filter(|c| c.status == status)
            .cloned()
            .collect()
    }

    /// Apply a config change (after approval if required)
    pub fn apply_change(&self, change_id: &ConfigChangeId) -> Result<()> {
        let mut changes = self.changes.lock().unwrap();
        let change = changes
            .get_mut(change_id)
            .ok_or_else(|| anyhow::anyhow!("Config change not found"))?;

        // Check if approval is required
        if let Some(approval_id) = &change.approval_id {
            let approval = self
                .approval_manager
                .get_approval(approval_id)
                .ok_or_else(|| anyhow::anyhow!("Approval not found"))?;

            if approval.status != crate::types::ApprovalStatus::Approved {
                return Err(anyhow::anyhow!(
                    "Change cannot be applied - not approved (status: {:?})",
                    approval.status
                ));
            }
        } else if change.status == ConfigChangeStatus::PendingApproval {
            return Err(anyhow::anyhow!(
                "Change is pending approval but no approval found"
            ));
        }

        // Mark as applied
        change.status = ConfigChangeStatus::Applied;
        change.applied_at = Some(Utc::now());

        tracing::info!("Applied config change {}: {}", change.id.0, change.description);

        Ok(())
    }

    /// Reject a config change
    pub fn reject_change(&self, change_id: &ConfigChangeId, reason: String) -> Result<()> {
        let mut changes = self.changes.lock().unwrap();
        let change = changes
            .get_mut(change_id)
            .ok_or_else(|| anyhow::anyhow!("Config change not found"))?;

        change.status = ConfigChangeStatus::Rejected;

        tracing::info!(
            "Rejected config change {}: {} - Reason: {}",
            change.id.0,
            change.description,
            reason
        );

        Ok(())
    }

    /// Mark a change as failed
    pub fn mark_failed(&self, change_id: &ConfigChangeId, error: String) -> Result<()> {
        let mut changes = self.changes.lock().unwrap();
        let change = changes
            .get_mut(change_id)
            .ok_or_else(|| anyhow::anyhow!("Config change not found"))?;

        change.status = ConfigChangeStatus::Failed;

        tracing::error!(
            "Config change {} failed: {} - Error: {}",
            change.id.0,
            change.description,
            error
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ApprovalBoard, PersonId, QuorumRule};

    fn create_test_approval_board() -> ApprovalBoard {
        ApprovalBoard {
            id: ApprovalBoardId::new("test_board"),
            name: "Test Board".to_string(),
            description: "Test approval board".to_string(),
            approvers: vec![PersonId::new("approver1"), PersonId::new("approver2")],
            quorum_rule: QuorumRule::Majority,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_propose_change_without_approval() {
        let approval_mgr = Arc::new(ApprovalManager::new());
        let change_mgr = ConfigChangeManager::new(approval_mgr);

        let change = change_mgr
            .propose_change(
                ConfigChangeType::Policy,
                "Update policy".to_string(),
                None,
                r#"{"new": "policy"}"#.to_string(),
                "admin".to_string(),
                None,
            )
            .unwrap();

        assert_eq!(change.status, ConfigChangeStatus::Proposed);
        assert!(change.approval_id.is_none());
    }

    #[test]
    fn test_propose_change_with_approval() {
        let approval_mgr = Arc::new(ApprovalManager::new());
        let board = create_test_approval_board();
        approval_mgr.register_board(board.clone()).unwrap();

        let change_mgr = ConfigChangeManager::new(approval_mgr);

        let change = change_mgr
            .propose_change(
                ConfigChangeType::Policy,
                "Update policy".to_string(),
                Some(r#"{"old": "policy"}"#.to_string()),
                r#"{"new": "policy"}"#.to_string(),
                "admin".to_string(),
                Some(board.id.clone()),
            )
            .unwrap();

        assert_eq!(change.status, ConfigChangeStatus::PendingApproval);
        assert!(change.approval_id.is_some());
    }

    #[test]
    fn test_apply_change_without_approval() {
        let approval_mgr = Arc::new(ApprovalManager::new());
        let change_mgr = ConfigChangeManager::new(approval_mgr);

        let change = change_mgr
            .propose_change(
                ConfigChangeType::Policy,
                "Update policy".to_string(),
                None,
                r#"{"new": "policy"}"#.to_string(),
                "admin".to_string(),
                None,
            )
            .unwrap();

        change_mgr.apply_change(&change.id).unwrap();

        let updated = change_mgr.get_change(&change.id).unwrap();
        assert_eq!(updated.status, ConfigChangeStatus::Applied);
        assert!(updated.applied_at.is_some());
    }

    #[test]
    fn test_apply_change_not_approved() {
        let approval_mgr = Arc::new(ApprovalManager::new());
        let board = create_test_approval_board();
        approval_mgr.register_board(board.clone()).unwrap();

        let change_mgr = ConfigChangeManager::new(approval_mgr);

        let change = change_mgr
            .propose_change(
                ConfigChangeType::Policy,
                "Update policy".to_string(),
                None,
                r#"{"new": "policy"}"#.to_string(),
                "admin".to_string(),
                Some(board.id.clone()),
            )
            .unwrap();

        // Should fail - not yet approved
        let result = change_mgr.apply_change(&change.id);
        assert!(result.is_err());
    }

    #[test]
    fn test_reject_change() {
        let approval_mgr = Arc::new(ApprovalManager::new());
        let change_mgr = ConfigChangeManager::new(approval_mgr);

        let change = change_mgr
            .propose_change(
                ConfigChangeType::Policy,
                "Update policy".to_string(),
                None,
                r#"{"new": "policy"}"#.to_string(),
                "admin".to_string(),
                None,
            )
            .unwrap();

        change_mgr
            .reject_change(&change.id, "Not needed".to_string())
            .unwrap();

        let updated = change_mgr.get_change(&change.id).unwrap();
        assert_eq!(updated.status, ConfigChangeStatus::Rejected);
    }

    #[test]
    fn test_list_changes_by_status() {
        let approval_mgr = Arc::new(ApprovalManager::new());
        let change_mgr = ConfigChangeManager::new(approval_mgr);

        // Create some changes
        change_mgr
            .propose_change(
                ConfigChangeType::Policy,
                "Change 1".to_string(),
                None,
                "{}".to_string(),
                "admin".to_string(),
                None,
            )
            .unwrap();

        let change2 = change_mgr
            .propose_change(
                ConfigChangeType::Role,
                "Change 2".to_string(),
                None,
                "{}".to_string(),
                "admin".to_string(),
                None,
            )
            .unwrap();

        change_mgr.apply_change(&change2.id).unwrap();

        let proposed = change_mgr.list_changes_by_status(ConfigChangeStatus::Proposed);
        assert_eq!(proposed.len(), 1);

        let applied = change_mgr.list_changes_by_status(ConfigChangeStatus::Applied);
        assert_eq!(applied.len(), 1);
    }
}
