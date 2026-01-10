//! Compliance API endpoints.

use crate::client::ShiiooClient;
use crate::error::ShiiooResult;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shiioo_core::compliance::{ComplianceFramework, ComplianceReport};

/// Compliance API for generating compliance reports.
pub struct ComplianceApi<'a> {
    client: &'a ShiiooClient,
}

impl<'a> ComplianceApi<'a> {
    pub(crate) fn new(client: &'a ShiiooClient) -> Self {
        Self { client }
    }

    /// Generate a compliance report.
    pub async fn generate_report(
        &self,
        request: ComplianceReportRequest,
    ) -> ShiiooResult<ComplianceReport> {
        self.client.http.post("/api/compliance/report", &request).await
    }
}

/// Request to generate a compliance report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceReportRequest {
    pub framework: ComplianceFramework,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
}
