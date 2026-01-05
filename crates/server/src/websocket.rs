use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::Response,
};
use axum::body::Bytes;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::config::AppState;

/// WebSocket message types for real-time updates
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsMessage {
    /// Workflow status update
    WorkflowUpdate {
        run_id: String,
        status: String,
        progress: f32,
        message: Option<String>,
    },
    /// Step status update
    StepUpdate {
        run_id: String,
        step_id: String,
        status: String,
        message: Option<String>,
    },
    /// Metrics update
    MetricsUpdate {
        metric_type: String,
        name: String,
        value: f64,
        labels: std::collections::HashMap<String, String>,
    },
    /// System health update
    HealthUpdate {
        status: String,
        active_workflows: usize,
        active_routines: usize,
        pending_approvals: usize,
    },
    /// Client subscription confirmation
    Subscribed { subscription_id: String },
    /// Error message
    Error { message: String },
    /// Ping/pong for connection keepalive
    Ping,
    Pong,
}

/// WebSocket subscription request
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum WsRequest {
    /// Subscribe to all workflows
    SubscribeAll,
    /// Subscribe to a specific workflow
    SubscribeWorkflow { run_id: String },
    /// Subscribe to metrics updates
    SubscribeMetrics,
    /// Subscribe to system health
    SubscribeHealth,
    /// Unsubscribe
    Unsubscribe,
}

/// WebSocket handler for real-time updates
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Handle individual WebSocket connection
async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>) {
    // Send initial connection confirmation
    let confirm_msg = WsMessage::Subscribed {
        subscription_id: uuid::Uuid::new_v4().to_string(),
    };

    if let Ok(msg_json) = serde_json::to_string(&confirm_msg) {
        let _ = socket.send(Message::Text(msg_json.into())).await;
    }

    // Handle incoming messages from client
    while let Some(msg_result) = socket.recv().await {
        match msg_result {
            Ok(Message::Text(text)) => {
                tracing::debug!("Received WS message: {}", text);

                // Parse request
                if let Ok(request) = serde_json::from_str::<WsRequest>(&text) {
                    match request {
                        WsRequest::SubscribeAll => {
                            tracing::info!("Client subscribed to all workflows");
                            let response = WsMessage::Subscribed {
                                subscription_id: "all_workflows".to_string(),
                            };
                            if let Ok(msg_json) = serde_json::to_string(&response) {
                                let _ = socket.send(Message::Text(msg_json.into())).await;
                            }
                        }
                        WsRequest::SubscribeWorkflow { run_id } => {
                            tracing::info!("Client subscribed to workflow: {}", run_id);
                        }
                        WsRequest::SubscribeMetrics => {
                            tracing::info!("Client subscribed to metrics");
                        }
                        WsRequest::SubscribeHealth => {
                            tracing::info!("Client subscribed to health updates");

                            // Send current health status
                            let workflows = state.routine_scheduler.list_routines();
                            let approvals = state.approval_manager.list_approvals();

                            let health_msg = WsMessage::HealthUpdate {
                                status: "healthy".to_string(),
                                active_workflows: 0,
                                active_routines: workflows.len(),
                                pending_approvals: approvals
                                    .iter()
                                    .filter(|a| {
                                        matches!(
                                            a.status,
                                            shiioo_core::types::ApprovalStatus::Pending
                                        )
                                    })
                                    .count(),
                            };

                            if let Ok(msg_json) = serde_json::to_string(&health_msg) {
                                let _ = socket.send(Message::Text(msg_json.into())).await;
                            }
                        }
                        WsRequest::Unsubscribe => {
                            tracing::info!("Client unsubscribed");
                            break;
                        }
                    }
                }
            }
            Ok(Message::Ping(_)) => {
                // Respond to ping with pong
                let _ = socket.send(Message::Pong(Bytes::new())).await;
            }
            Ok(Message::Close(_)) => {
                tracing::info!("WebSocket connection closed");
                break;
            }
            Err(e) => {
                tracing::error!("WebSocket error: {}", e);
                break;
            }
            _ => {}
        }
    }

    tracing::info!("WebSocket connection terminated");
}
