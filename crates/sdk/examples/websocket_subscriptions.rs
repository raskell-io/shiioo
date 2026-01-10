//! Example: Real-time updates via WebSocket subscriptions.
//!
//! This example demonstrates how to subscribe to real-time updates
//! for workflow executions, metrics, and health status.
//!
//! Run with: cargo run --example websocket_subscriptions

use shiioo_sdk::{stream::SubscriptionEvent, ShiiooClient, ShiiooResult};

#[tokio::main]
async fn main() -> ShiiooResult<()> {
    tracing_subscriber::fmt::init();

    let client = ShiiooClient::builder()
        .base_url("http://localhost:8080")
        .build()?;

    println!("Connecting to WebSocket...");
    let mut subscription = client.subscribe().await?;

    // Subscribe to different event types
    println!("Subscribing to all workflow updates...");
    subscription.subscribe_all().await?;

    println!("Subscribing to health updates...");
    subscription.subscribe_health().await?;

    println!("Subscribing to metrics updates...");
    subscription.subscribe_metrics().await?;

    println!("\nListening for events (Ctrl+C to stop)...\n");

    // Process incoming events
    while let Some(result) = subscription.next_event().await {
        match result {
            Ok(event) => match event {
                SubscriptionEvent::WorkflowUpdate {
                    run_id,
                    status,
                    progress,
                    message,
                } => {
                    println!(
                        "[WORKFLOW] Run {}: {} ({:.0}%)",
                        run_id,
                        status,
                        progress * 100.0
                    );
                    if let Some(msg) = message {
                        println!("           {}", msg);
                    }
                }

                SubscriptionEvent::StepUpdate {
                    run_id,
                    step_id,
                    status,
                    message,
                } => {
                    println!("[STEP] Run {} / Step {}: {}", run_id, step_id, status);
                    if let Some(msg) = message {
                        println!("       {}", msg);
                    }
                }

                SubscriptionEvent::HealthUpdate {
                    status,
                    active_workflows,
                    active_routines,
                    pending_approvals,
                } => {
                    println!(
                        "[HEALTH] Status: {}, Workflows: {}, Routines: {}, Pending Approvals: {}",
                        status, active_workflows, active_routines, pending_approvals
                    );
                }

                SubscriptionEvent::MetricsUpdate {
                    metric_type,
                    name,
                    value,
                    labels,
                } => {
                    println!(
                        "[METRICS] {}/{}: {} {:?}",
                        metric_type, name, value, labels
                    );
                }

                SubscriptionEvent::Subscribed { subscription_id } => {
                    println!("[SUBSCRIBED] ID: {}", subscription_id);
                }

                SubscriptionEvent::Error { message } => {
                    eprintln!("[ERROR] {}", message);
                }

                SubscriptionEvent::Ping => {
                    println!("[PING]");
                }

                SubscriptionEvent::Pong => {
                    println!("[PONG]");
                }
            },
            Err(e) => {
                eprintln!("Error receiving event: {}", e);
                break;
            }
        }
    }

    println!("\nWebSocket connection closed.");
    Ok(())
}
