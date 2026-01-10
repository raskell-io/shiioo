//! Example: Working with approvals and approval boards.
//!
//! This example demonstrates how to manage approval workflows,
//! create approval boards, and cast votes.
//!
//! Run with: cargo run --example approvals

use shiioo_sdk::{
    ApprovalBoard, ApprovalBoardId, ApprovalStatus, PersonId, QuorumRule, VoteDecision,
    api::approvals::CastVoteRequest,
    ShiiooClient, ShiiooResult,
};

#[tokio::main]
async fn main() -> ShiiooResult<()> {
    tracing_subscriber::fmt::init();

    let client = ShiiooClient::builder()
        .base_url("http://localhost:8080")
        .build()?;

    // Create an approval board
    println!("Creating approval board...");
    let board = ApprovalBoard {
        id: ApprovalBoardId::new("security-review-board"),
        name: "Security Review Board".to_string(),
        description: "Reviews security-sensitive changes".to_string(),
        approvers: vec![
            PersonId::new("alice"),
            PersonId::new("bob"),
            PersonId::new("charlie"),
        ],
        quorum_rule: QuorumRule::Majority,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    let response = client.approval_boards().create(&board).await?;
    println!("Created board: {} - {}", response.board_id, response.message);

    // List all approval boards
    println!("\nListing approval boards...");
    let boards = client.approval_boards().list().await?;
    for b in &boards {
        println!(
            "  {} ({}) - {} approvers, quorum: {:?}",
            b.name,
            b.id.0,
            b.approvers.len(),
            b.quorum_rule
        );
    }

    // List pending approvals
    println!("\nListing pending approvals...");
    let approvals = client.approvals().list().await?;
    let pending: Vec<_> = approvals
        .iter()
        .filter(|a| matches!(a.status, ApprovalStatus::Pending))
        .collect();

    println!("Found {} pending approvals", pending.len());

    for approval in &pending {
        println!("\nApproval: {}", approval.id.0);
        println!("  Subject: {:?}", approval.subject);
        println!("  Board: {}", approval.board_id.0);
        println!("  Votes: {}", approval.votes.len());
        println!("  Status: {:?}", approval.status);

        // Show existing votes
        for vote in &approval.votes {
            println!(
                "    - {} voted {:?}{}",
                vote.voter.0,
                vote.vote,
                vote.comment
                    .as_ref()
                    .map(|c| format!(": {}", c))
                    .unwrap_or_default()
            );
        }
    }

    // Cast a vote on the first pending approval (if any)
    if let Some(approval) = pending.first() {
        println!("\nCasting vote on approval {}...", approval.id.0);

        let vote_response = client
            .approvals()
            .vote(
                &approval.id,
                CastVoteRequest {
                    voter_id: PersonId::new("alice"),
                    decision: VoteDecision::Approve,
                    comment: Some("Looks good to me!".to_string()),
                },
            )
            .await?;

        println!("Vote cast: {}", vote_response.message);

        // Check updated status
        let updated = client.approvals().get(&approval.id).await?;
        println!("Updated status: {:?}", updated.status);
        println!("Votes: {}", updated.votes.len());
    }

    println!("\nApprovals example completed!");
    Ok(())
}
