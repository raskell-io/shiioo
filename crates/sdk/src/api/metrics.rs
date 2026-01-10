//! Metrics API endpoints.

use crate::client::ShiiooClient;
use crate::error::ShiiooResult;
use serde::{Deserialize, Serialize};
use shiioo_core::metrics::{Counter, Gauge, Histogram};

/// Metrics API for retrieving system metrics.
pub struct MetricsApi<'a> {
    client: &'a ShiiooClient,
}

impl<'a> MetricsApi<'a> {
    pub(crate) fn new(client: &'a ShiiooClient) -> Self {
        Self { client }
    }

    /// Get all metrics.
    pub async fn get(&self) -> ShiiooResult<MetricsResponse> {
        self.client.http.get("/api/metrics").await
    }
}

/// Response containing all metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsResponse {
    pub counters: Vec<Counter>,
    pub gauges: Vec<Gauge>,
    pub histograms: Vec<Histogram>,
}
