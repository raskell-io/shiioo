//! WebSocket transport for real-time subscriptions.

use crate::config::ClientConfig;
use crate::error::{ShiiooError, ShiiooResult};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info};

/// WebSocket client for real-time subscriptions.
pub struct WebSocketClient {
    config: Arc<ClientConfig>,
    sender: Option<mpsc::Sender<WsRequest>>,
    receiver: Option<mpsc::Receiver<ShiiooResult<SubscriptionEvent>>>,
}

impl WebSocketClient {
    /// Create a new WebSocket client.
    pub fn new(config: Arc<ClientConfig>) -> Self {
        Self {
            config,
            sender: None,
            receiver: None,
        }
    }

    /// Connect to the WebSocket endpoint.
    pub async fn connect(&mut self) -> ShiiooResult<()> {
        let ws_url = self.build_ws_url()?;
        debug!(url = %ws_url, "Connecting to WebSocket");

        let (ws_stream, _) = connect_async(&ws_url)
            .await
            .map_err(|e| ShiiooError::WebSocket(e.to_string()))?;

        let (mut write, mut read) = ws_stream.split();

        // Channel for sending requests to the WebSocket
        let (request_tx, mut request_rx) = mpsc::channel::<WsRequest>(32);

        // Channel for receiving events from the WebSocket
        let (event_tx, event_rx) = mpsc::channel::<ShiiooResult<SubscriptionEvent>>(128);

        // Spawn task to handle outgoing messages
        tokio::spawn(async move {
            while let Some(request) = request_rx.recv().await {
                let json = match serde_json::to_string(&request) {
                    Ok(j) => j,
                    Err(e) => {
                        error!(error = %e, "Failed to serialize WebSocket request");
                        continue;
                    }
                };
                if let Err(e) = write.send(Message::Text(json)).await {
                    error!(error = %e, "Failed to send WebSocket message");
                    break;
                }
            }
        });

        // Spawn task to handle incoming messages
        let event_tx_clone = event_tx.clone();
        tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        let event = serde_json::from_str::<SubscriptionEvent>(&text)
                            .map_err(|e| ShiiooError::Json(e));
                        if event_tx_clone.send(event).await.is_err() {
                            break;
                        }
                    }
                    Ok(Message::Ping(_)) => {
                        debug!("Received ping");
                        // Pong will be handled by the library
                    }
                    Ok(Message::Close(_)) => {
                        info!("WebSocket connection closed");
                        break;
                    }
                    Err(e) => {
                        let _ = event_tx_clone
                            .send(Err(ShiiooError::WebSocket(e.to_string())))
                            .await;
                        break;
                    }
                    _ => {}
                }
            }
        });

        self.sender = Some(request_tx);
        self.receiver = Some(event_rx);

        info!("WebSocket connected");
        Ok(())
    }

    /// Build the WebSocket URL from the base URL.
    fn build_ws_url(&self) -> ShiiooResult<String> {
        let mut url = self.config.base_url.clone();

        // Change scheme to ws/wss
        let new_scheme = match url.scheme() {
            "http" => "ws",
            "https" => "wss",
            _ => "ws",
        };

        url.set_scheme(new_scheme)
            .map_err(|_| ShiiooError::Config("Failed to set WebSocket scheme".to_string()))?;

        url.set_path("/api/ws");

        // Add API key as query parameter if present
        if let Some(ref api_key) = self.config.api_key {
            url.query_pairs_mut().append_pair("token", api_key);
        }

        Ok(url.to_string())
    }

    /// Send a request to the WebSocket.
    async fn send_request(&self, request: WsRequest) -> ShiiooResult<()> {
        let sender = self
            .sender
            .as_ref()
            .ok_or_else(|| ShiiooError::WebSocket("Not connected".to_string()))?;

        sender
            .send(request)
            .await
            .map_err(|_| ShiiooError::WebSocket("Failed to send request".to_string()))
    }

    /// Subscribe to all workflow updates.
    pub async fn subscribe_all(&self) -> ShiiooResult<()> {
        self.send_request(WsRequest::SubscribeAll).await
    }

    /// Subscribe to updates for a specific workflow run.
    pub async fn subscribe_workflow(&self, run_id: &str) -> ShiiooResult<()> {
        self.send_request(WsRequest::SubscribeWorkflow {
            run_id: run_id.to_string(),
        })
        .await
    }

    /// Subscribe to metrics updates.
    pub async fn subscribe_metrics(&self) -> ShiiooResult<()> {
        self.send_request(WsRequest::SubscribeMetrics).await
    }

    /// Subscribe to health updates.
    pub async fn subscribe_health(&self) -> ShiiooResult<()> {
        self.send_request(WsRequest::SubscribeHealth).await
    }

    /// Unsubscribe from all subscriptions.
    pub async fn unsubscribe(&self) -> ShiiooResult<()> {
        self.send_request(WsRequest::Unsubscribe).await
    }

    /// Get the next event from the subscription.
    pub async fn next_event(&mut self) -> Option<ShiiooResult<SubscriptionEvent>> {
        self.receiver.as_mut()?.recv().await
    }
}

/// WebSocket request types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsRequest {
    SubscribeAll,
    SubscribeWorkflow { run_id: String },
    SubscribeMetrics,
    SubscribeHealth,
    Unsubscribe,
}

/// Events received from WebSocket subscriptions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SubscriptionEvent {
    /// Workflow status update.
    WorkflowUpdate {
        run_id: String,
        status: String,
        progress: f32,
        message: Option<String>,
    },
    /// Step status update.
    StepUpdate {
        run_id: String,
        step_id: String,
        status: String,
        message: Option<String>,
    },
    /// Metrics update.
    MetricsUpdate {
        metric_type: String,
        name: String,
        value: f64,
        labels: HashMap<String, String>,
    },
    /// Health status update.
    HealthUpdate {
        status: String,
        active_workflows: usize,
        active_routines: usize,
        pending_approvals: usize,
    },
    /// Subscription confirmed.
    Subscribed { subscription_id: String },
    /// Error from server.
    Error { message: String },
    /// Ping from server.
    Ping,
    /// Pong response.
    Pong,
}
