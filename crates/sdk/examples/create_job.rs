//! Example: Creating and executing a workflow job.
//!
//! This example demonstrates how to create a job with a workflow specification
//! and monitor its execution.
//!
//! Run with: cargo run --example create_job

use shiioo_sdk::{
    api::jobs::CreateJobRequest,
    RetryPolicy, RoleId, StepAction, StepId, StepSpec, WorkflowSpec,
    ShiiooClient, ShiiooResult,
};
use std::collections::HashMap;
use std::time::Duration;

#[tokio::main]
async fn main() -> ShiiooResult<()> {
    tracing_subscriber::fmt::init();

    let client = ShiiooClient::builder()
        .base_url("http://localhost:8080")
        .build()?;

    // Define a simple workflow with two steps
    let workflow = WorkflowSpec {
        steps: vec![
            StepSpec {
                id: StepId::new("analyze"),
                name: "Analyze Code".to_string(),
                description: Some("Analyze the codebase for issues".to_string()),
                role: RoleId::new("code-analyst"),
                action: StepAction::AgentTask {
                    prompt: "Analyze the provided code for potential bugs and improvements.".to_string(),
                },
                timeout_secs: Some(300),
                retry_policy: Some(RetryPolicy { max_attempts: 3, backoff_secs: 2 }),
                requires_approval: false,
            },
            StepSpec {
                id: StepId::new("report"),
                name: "Generate Report".to_string(),
                description: Some("Generate a summary report".to_string()),
                role: RoleId::new("reporter"),
                action: StepAction::AgentTask {
                    prompt: "Generate a markdown report summarizing the analysis findings.".to_string(),
                },
                timeout_secs: Some(120),
                retry_policy: Some(RetryPolicy { max_attempts: 2, backoff_secs: 2 }),
                requires_approval: false,
            },
        ],
        dependencies: {
            let mut deps = HashMap::new();
            deps.insert(StepId::new("report"), vec![StepId::new("analyze")]);
            deps
        },
    };

    // Create the job
    println!("Creating job...");
    let response = client
        .jobs()
        .create(CreateJobRequest {
            name: "Code Analysis Job".to_string(),
            description: Some("Automated code analysis workflow".to_string()),
            workflow,
            created_by: Some("sdk-example".to_string()),
            execute: Some(true), // Execute immediately
        })
        .await?;

    println!("Job created: {}", response.job_id);
    println!("Message: {}", response.message);

    if let Some(run_id) = &response.run_id {
        println!("Run ID: {}", run_id.0);

        // Poll for run status
        println!("\nMonitoring execution...");
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;

            let run = client.runs().get(run_id).await?;
            println!(
                "  Status: {:?}, Steps: {}/{}",
                run.status,
                run.steps.iter().filter(|s| matches!(s.status, shiioo_sdk::StepStatus::Completed)).count(),
                run.steps.len()
            );

            match run.status {
                shiioo_sdk::RunStatus::Completed => {
                    println!("\nWorkflow completed successfully!");
                    break;
                }
                shiioo_sdk::RunStatus::Failed => {
                    println!("\nWorkflow failed!");
                    // Get events to see what went wrong
                    let events = client.runs().events(run_id).await?;
                    for event in events.iter().rev().take(5) {
                        println!("  Event: {:?}", event.event_type);
                    }
                    break;
                }
                shiioo_sdk::RunStatus::Cancelled => {
                    println!("\nWorkflow was cancelled.");
                    break;
                }
                _ => continue,
            }
        }
    }

    Ok(())
}
