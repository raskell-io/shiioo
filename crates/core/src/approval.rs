use crate::types::{
    Approval, ApprovalBoard, ApprovalBoardId, ApprovalId, ApprovalStatus, ApprovalSubject,
    ApprovalVote, PersonId, QuorumRule, VoteDecision,
};
use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Approval board manager
pub struct ApprovalManager {
    boards: Arc<Mutex<HashMap<ApprovalBoardId, ApprovalBoard>>>,
    approvals: Arc<Mutex<HashMap<ApprovalId, Approval>>>,
}

impl ApprovalManager {
    /// Create a new approval manager
    pub fn new() -> Self {
        Self {
            boards: Arc::new(Mutex::new(HashMap::new())),
            approvals: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register an approval board
    pub fn register_board(&self, board: ApprovalBoard) -> Result<()> {
        self.boards.lock().unwrap().insert(board.id.clone(), board);
        Ok(())
    }

    /// Get all boards
    pub fn list_boards(&self) -> Vec<ApprovalBoard> {
        self.boards.lock().unwrap().values().cloned().collect()
    }

    /// Get a specific board
    pub fn get_board(&self, board_id: &ApprovalBoardId) -> Option<ApprovalBoard> {
        self.boards.lock().unwrap().get(board_id).cloned()
    }

    /// Delete a board
    pub fn delete_board(&self, board_id: &ApprovalBoardId) -> Result<()> {
        self.boards.lock().unwrap().remove(board_id);
        Ok(())
    }

    /// Create an approval request
    pub fn create_approval(
        &self,
        board_id: ApprovalBoardId,
        subject: ApprovalSubject,
        created_by: String,
    ) -> Result<Approval> {
        // Verify board exists
        let board = self
            .get_board(&board_id)
            .ok_or_else(|| anyhow::anyhow!("Approval board not found"))?;

        let approval = Approval {
            id: ApprovalId::new(uuid::Uuid::new_v4().to_string()),
            board_id,
            subject,
            status: ApprovalStatus::Pending,
            votes: Vec::new(),
            created_at: Utc::now(),
            created_by,
            resolved_at: None,
        };

        self.approvals
            .lock()
            .unwrap()
            .insert(approval.id.clone(), approval.clone());

        tracing::info!(
            "Created approval {} on board {} with {} approvers",
            approval.id.0,
            board.name,
            board.approvers.len()
        );

        Ok(approval)
    }

    /// Cast a vote on an approval
    pub fn cast_vote(
        &self,
        approval_id: &ApprovalId,
        voter: PersonId,
        vote: VoteDecision,
        comment: Option<String>,
    ) -> Result<ApprovalStatus> {
        let mut approvals = self.approvals.lock().unwrap();
        let approval = approvals
            .get_mut(approval_id)
            .ok_or_else(|| anyhow::anyhow!("Approval not found"))?;

        // Check if already resolved
        if approval.status != ApprovalStatus::Pending {
            return Err(anyhow::anyhow!("Approval already resolved"));
        }

        // Check if voter is on the board
        let board = self
            .get_board(&approval.board_id)
            .ok_or_else(|| anyhow::anyhow!("Approval board not found"))?;

        if !board.approvers.contains(&voter) {
            return Err(anyhow::anyhow!("Voter is not an approver on this board"));
        }

        // Check if voter already voted
        if approval.votes.iter().any(|v| v.voter == voter) {
            return Err(anyhow::anyhow!("Voter has already voted"));
        }

        // Add the vote
        approval.votes.push(ApprovalVote {
            voter,
            vote,
            comment,
            voted_at: Utc::now(),
        });

        // Check if quorum is met
        let result = self.check_quorum(&board, &approval.votes)?;
        if result != ApprovalStatus::Pending {
            approval.status = result;
            approval.resolved_at = Some(Utc::now());
            tracing::info!(
                "Approval {} resolved with status: {:?}",
                approval.id.0,
                result
            );
        }

        Ok(result)
    }

    /// Check if quorum is met
    fn check_quorum(
        &self,
        board: &ApprovalBoard,
        votes: &[ApprovalVote],
    ) -> Result<ApprovalStatus> {
        let total_approvers = board.approvers.len();
        let approve_votes = votes.iter().filter(|v| v.vote == VoteDecision::Approve).count();
        let reject_votes = votes.iter().filter(|v| v.vote == VoteDecision::Reject).count();
        let abstain_votes = votes.iter().filter(|v| v.vote == VoteDecision::Abstain).count();

        match &board.quorum_rule {
            QuorumRule::Unanimous => {
                // All approvers must approve
                if approve_votes == total_approvers {
                    Ok(ApprovalStatus::Approved)
                } else if reject_votes > 0 {
                    Ok(ApprovalStatus::Denied)
                } else {
                    Ok(ApprovalStatus::Pending)
                }
            }
            QuorumRule::Majority => {
                // More than 50% must approve
                let required = (total_approvers / 2) + 1;
                if approve_votes >= required {
                    Ok(ApprovalStatus::Approved)
                } else if reject_votes >= required {
                    Ok(ApprovalStatus::Denied)
                } else if approve_votes + reject_votes + abstain_votes == total_approvers {
                    // All voted but no quorum
                    Ok(ApprovalStatus::Denied)
                } else {
                    Ok(ApprovalStatus::Pending)
                }
            }
            QuorumRule::MinCount { min } => {
                // At least N approvers must approve
                if approve_votes >= *min as usize {
                    Ok(ApprovalStatus::Approved)
                } else if (total_approvers - reject_votes) < *min as usize {
                    // Not enough approvers left to reach quorum
                    Ok(ApprovalStatus::Denied)
                } else {
                    Ok(ApprovalStatus::Pending)
                }
            }
            QuorumRule::Percentage { percent } => {
                // At least X% of approvers must approve
                let required = ((total_approvers as f64 * (*percent as f64 / 100.0)).ceil()) as usize;
                if approve_votes >= required {
                    Ok(ApprovalStatus::Approved)
                } else if (total_approvers - reject_votes) < required {
                    // Not enough approvers left to reach quorum
                    Ok(ApprovalStatus::Denied)
                } else {
                    Ok(ApprovalStatus::Pending)
                }
            }
        }
    }

    /// List all approvals
    pub fn list_approvals(&self) -> Vec<Approval> {
        self.approvals.lock().unwrap().values().cloned().collect()
    }

    /// Get a specific approval
    pub fn get_approval(&self, approval_id: &ApprovalId) -> Option<Approval> {
        self.approvals.lock().unwrap().get(approval_id).cloned()
    }

    /// List pending approvals for a board
    pub fn list_pending_approvals(&self, board_id: &ApprovalBoardId) -> Vec<Approval> {
        self.approvals
            .lock()
            .unwrap()
            .values()
            .filter(|a| a.board_id == *board_id && a.status == ApprovalStatus::Pending)
            .cloned()
            .collect()
    }

    /// List approvals for a person
    pub fn list_approvals_for_person(&self, person_id: &PersonId) -> Vec<Approval> {
        let boards = self.boards.lock().unwrap();
        let approvals = self.approvals.lock().unwrap();

        approvals
            .values()
            .filter(|a| {
                if let Some(board) = boards.get(&a.board_id) {
                    board.approvers.contains(person_id)
                } else {
                    false
                }
            })
            .cloned()
            .collect()
    }
}

impl Default for ApprovalManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ConfigChangeId;

    fn create_test_board() -> ApprovalBoard {
        ApprovalBoard {
            id: ApprovalBoardId::new("test_board"),
            name: "Test Board".to_string(),
            description: "A test approval board".to_string(),
            approvers: vec![
                PersonId::new("approver1"),
                PersonId::new("approver2"),
                PersonId::new("approver3"),
            ],
            quorum_rule: QuorumRule::Majority,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_register_board() {
        let manager = ApprovalManager::new();
        let board = create_test_board();

        manager.register_board(board.clone()).unwrap();

        let retrieved = manager.get_board(&board.id).unwrap();
        assert_eq!(retrieved.id, board.id);
        assert_eq!(retrieved.approvers.len(), 3);
    }

    #[test]
    fn test_create_approval() {
        let manager = ApprovalManager::new();
        let board = create_test_board();
        manager.register_board(board.clone()).unwrap();

        let approval = manager
            .create_approval(
                board.id.clone(),
                ApprovalSubject::ConfigChange {
                    change_id: ConfigChangeId::new("test_change"),
                },
                "admin".to_string(),
            )
            .unwrap();

        assert_eq!(approval.board_id, board.id);
        assert_eq!(approval.status, ApprovalStatus::Pending);
        assert_eq!(approval.votes.len(), 0);
    }

    #[test]
    fn test_majority_quorum() {
        let manager = ApprovalManager::new();
        let board = create_test_board();
        manager.register_board(board.clone()).unwrap();

        let approval = manager
            .create_approval(
                board.id.clone(),
                ApprovalSubject::ConfigChange {
                    change_id: ConfigChangeId::new("test_change"),
                },
                "admin".to_string(),
            )
            .unwrap();

        // First vote - still pending
        let status = manager
            .cast_vote(
                &approval.id,
                PersonId::new("approver1"),
                VoteDecision::Approve,
                None,
            )
            .unwrap();
        assert_eq!(status, ApprovalStatus::Pending);

        // Second vote - should approve (2 out of 3 = majority)
        let status = manager
            .cast_vote(
                &approval.id,
                PersonId::new("approver2"),
                VoteDecision::Approve,
                None,
            )
            .unwrap();
        assert_eq!(status, ApprovalStatus::Approved);
    }

    #[test]
    fn test_unanimous_quorum() {
        let manager = ApprovalManager::new();
        let mut board = create_test_board();
        board.quorum_rule = QuorumRule::Unanimous;
        manager.register_board(board.clone()).unwrap();

        let approval = manager
            .create_approval(
                board.id.clone(),
                ApprovalSubject::ConfigChange {
                    change_id: ConfigChangeId::new("test_change"),
                },
                "admin".to_string(),
            )
            .unwrap();

        // First two approvals - still pending
        manager
            .cast_vote(
                &approval.id,
                PersonId::new("approver1"),
                VoteDecision::Approve,
                None,
            )
            .unwrap();
        let status = manager
            .cast_vote(
                &approval.id,
                PersonId::new("approver2"),
                VoteDecision::Approve,
                None,
            )
            .unwrap();
        assert_eq!(status, ApprovalStatus::Pending);

        // Third approval - should approve
        let status = manager
            .cast_vote(
                &approval.id,
                PersonId::new("approver3"),
                VoteDecision::Approve,
                None,
            )
            .unwrap();
        assert_eq!(status, ApprovalStatus::Approved);
    }

    #[test]
    fn test_rejection() {
        let manager = ApprovalManager::new();
        let board = create_test_board();
        manager.register_board(board.clone()).unwrap();

        let approval = manager
            .create_approval(
                board.id.clone(),
                ApprovalSubject::ConfigChange {
                    change_id: ConfigChangeId::new("test_change"),
                },
                "admin".to_string(),
            )
            .unwrap();

        // First vote approve
        manager
            .cast_vote(
                &approval.id,
                PersonId::new("approver1"),
                VoteDecision::Approve,
                None,
            )
            .unwrap();

        // Second vote reject - should still be pending (1-1)
        let status = manager
            .cast_vote(
                &approval.id,
                PersonId::new("approver2"),
                VoteDecision::Reject,
                None,
            )
            .unwrap();
        assert_eq!(status, ApprovalStatus::Pending);

        // Third vote reject - should deny (1-2)
        let status = manager
            .cast_vote(
                &approval.id,
                PersonId::new("approver3"),
                VoteDecision::Reject,
                None,
            )
            .unwrap();
        assert_eq!(status, ApprovalStatus::Denied);
    }

    #[test]
    fn test_min_count_quorum() {
        let manager = ApprovalManager::new();
        let mut board = create_test_board();
        board.quorum_rule = QuorumRule::MinCount { min: 2 };
        manager.register_board(board.clone()).unwrap();

        let approval = manager
            .create_approval(
                board.id.clone(),
                ApprovalSubject::ConfigChange {
                    change_id: ConfigChangeId::new("test_change"),
                },
                "admin".to_string(),
            )
            .unwrap();

        // First vote - still pending
        let status = manager
            .cast_vote(
                &approval.id,
                PersonId::new("approver1"),
                VoteDecision::Approve,
                None,
            )
            .unwrap();
        assert_eq!(status, ApprovalStatus::Pending);

        // Second vote - should approve (reached min count of 2)
        let status = manager
            .cast_vote(
                &approval.id,
                PersonId::new("approver2"),
                VoteDecision::Approve,
                None,
            )
            .unwrap();
        assert_eq!(status, ApprovalStatus::Approved);
    }

    #[test]
    fn test_percentage_quorum() {
        let manager = ApprovalManager::new();
        let mut board = create_test_board();
        board.quorum_rule = QuorumRule::Percentage { percent: 66 }; // 66% of 3 = ceil(1.98) = 2
        manager.register_board(board.clone()).unwrap();

        let approval = manager
            .create_approval(
                board.id.clone(),
                ApprovalSubject::ConfigChange {
                    change_id: ConfigChangeId::new("test_change"),
                },
                "admin".to_string(),
            )
            .unwrap();

        // First vote - still pending
        let status = manager
            .cast_vote(
                &approval.id,
                PersonId::new("approver1"),
                VoteDecision::Approve,
                None,
            )
            .unwrap();
        assert_eq!(status, ApprovalStatus::Pending);

        // Second vote - should approve (2/3 = 67%)
        let status = manager
            .cast_vote(
                &approval.id,
                PersonId::new("approver2"),
                VoteDecision::Approve,
                None,
            )
            .unwrap();
        assert_eq!(status, ApprovalStatus::Approved);
    }

    #[test]
    fn test_duplicate_vote() {
        let manager = ApprovalManager::new();
        let board = create_test_board();
        manager.register_board(board.clone()).unwrap();

        let approval = manager
            .create_approval(
                board.id.clone(),
                ApprovalSubject::ConfigChange {
                    change_id: ConfigChangeId::new("test_change"),
                },
                "admin".to_string(),
            )
            .unwrap();

        // First vote
        manager
            .cast_vote(
                &approval.id,
                PersonId::new("approver1"),
                VoteDecision::Approve,
                None,
            )
            .unwrap();

        // Duplicate vote should fail
        let result = manager.cast_vote(
            &approval.id,
            PersonId::new("approver1"),
            VoteDecision::Approve,
            None,
        );
        assert!(result.is_err());
    }
}
