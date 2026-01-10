//! Example: Managing scheduled routines.
//!
//! This example demonstrates how to create, manage, and monitor
//! scheduled workflow routines (cron jobs).
//!
//! Run with: cargo run --example routines

use shiioo_sdk::{
    api::routines::CreateRoutineRequest,
    RetryPolicy, RoleId, RoutineSchedule, StepAction, StepId, StepSpec, WorkflowSpec,
    ShiiooClient, ShiiooResult,
};
use std::collections::HashMap;

#[tokio::main]
async fn main() -> ShiiooResult<()> {
    tracing_subscriber::fmt::init();

    let client = ShiiooClient::builder()
        .base_url("http://localhost:8080")
        .build()?;

    // Define a simple daily report workflow
    let workflow = WorkflowSpec {
        steps: vec![StepSpec {
            id: StepId::new("daily-report"),
            name: "Generate Daily Report".to_string(),
            description: Some("Generate and send daily status report".to_string()),
            role: RoleId::new("reporter"),
            action: StepAction::AgentTask {
                prompt: "Generate a daily status report summarizing system health and activity.".to_string(),
            },
            timeout_secs: Some(300),
            retry_policy: Some(RetryPolicy { max_attempts: 3, backoff_secs: 5 }),
            requires_approval: false,
        }],
        dependencies: HashMap::new(),
    };

    // Create a routine that runs daily at 9 AM
    println!("Creating scheduled routine...");
    let response = client
        .routines()
        .create(CreateRoutineRequest {
            name: "Daily Status Report".to_string(),
            description: "Generates and sends daily status report every morning".to_string(),
            schedule: RoutineSchedule {
                cron: "0 9 * * *".to_string(), // Every day at 9:00 AM
                timezone: "UTC".to_string(),
            },
            workflow,
            enabled: Some(true),
            created_by: Some("sdk-example".to_string()),
        })
        .await?;

    println!("Created routine: {} - {}", response.routine_id, response.message);

    // List all routines
    println!("\nListing all routines...");
    let routines = client.routines().list().await?;
    println!("Found {} routines", routines.len());

    for routine in &routines {
        println!(
            "\n  {} ({})",
            routine.name,
            routine.id.0
        );
        println!("    Enabled: {}", routine.enabled);
        println!("    Schedule: {:?}", routine.schedule);
        println!(
            "    Last run: {}",
            routine
                .last_run
                .map(|t| t.to_string())
                .unwrap_or_else(|| "never".to_string())
        );
        println!("    Next run: {}", routine.next_run);
    }

    // Get execution history for a routine
    if let Some(routine) = routines.first() {
        println!("\nGetting execution history for '{}'...", routine.name);
        let executions = client.routines().executions(&routine.id).await?;
        println!("Found {} executions", executions.len());

        for exec in executions.iter().take(5) {
            println!(
                "  {} - {:?} (run: {})",
                exec.executed_at,
                exec.status,
                exec.run_id.0
            );
        }

        // Toggle routine status
        if routine.enabled {
            println!("\nDisabling routine...");
            client.routines().disable(&routine.id).await?;
            println!("Routine disabled.");

            println!("Re-enabling routine...");
            client.routines().enable(&routine.id).await?;
            println!("Routine enabled.");
        }
    }

    println!("\nRoutines example completed!");
    Ok(())
}
