//! Basic SDK usage example.
//!
//! This example demonstrates how to connect to a Shiioo server and perform
//! basic operations like checking health and listing runs.
//!
//! Run with: cargo run --example basic_usage

use shiioo_sdk::{ShiiooClient, ShiiooResult};
use std::time::Duration;

#[tokio::main]
async fn main() -> ShiiooResult<()> {
    // Initialize tracing for debug output
    tracing_subscriber::fmt::init();

    // Build the client with configuration
    let client = ShiiooClient::builder()
        .base_url("http://localhost:8080")
        .api_key("sk-your-api-key") // Optional: for authenticated endpoints
        .timeout(Duration::from_secs(30))
        .build()?;

    // Check server health
    println!("Checking server health...");
    let health = client.health().check().await?;
    println!("Server status: {}", health.status);

    // Get detailed health status
    let status = client.health().status().await?;
    println!("\nDetailed Health Status:");
    println!("  Uptime: {} seconds", status.uptime_secs);
    println!("  Active routines: {}/{}", status.active_routines, status.total_routines);
    println!("  Pending approvals: {}", status.pending_approvals);
    println!("  Total executions: {}", status.total_workflow_executions);
    println!("  Success rate: {:.1}%", status.success_rate);

    // List all workflow runs
    println!("\nListing workflow runs...");
    let runs = client.runs().list().await?;
    println!("Found {} runs", runs.len());

    for run in runs.iter().take(5) {
        println!(
            "  Run {}: {:?} (started: {})",
            run.id.0,
            run.status,
            run.started_at
        );
    }

    // List roles
    println!("\nListing roles...");
    let roles = client.roles().list().await?;
    println!("Found {} roles", roles.len());

    for role in roles.iter().take(5) {
        println!("  Role: {} ({})", role.name, role.id.0);
    }

    // List policies
    println!("\nListing policies...");
    let policies = client.policies().list().await?;
    println!("Found {} policies", policies.len());

    for policy in policies.iter().take(5) {
        println!("  Policy: {} ({})", policy.name, policy.id.0);
    }

    println!("\nBasic usage example completed successfully!");
    Ok(())
}
