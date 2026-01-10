//! Audit API endpoints.

use crate::client::ShiiooClient;
use crate::error::ShiiooResult;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shiioo_core::audit::{AuditCategory, AuditEntry, AuditStatistics};

/// Audit API for audit log access.
pub struct AuditApi<'a> {
    client: &'a ShiiooClient,
}

impl<'a> AuditApi<'a> {
    pub(crate) fn new(client: &'a ShiiooClient) -> Self {
        Self { client }
    }

    /// List all audit entries.
    pub async fn entries(&self) -> ShiiooResult<Vec<AuditEntry>> {
        self.client.http.get("/api/audit/entries").await
    }

    /// List audit entries with filters.
    pub async fn entries_with_filter(&self, filter: AuditFilter) -> ShiiooResult<Vec<AuditEntry>> {
        self.client
            .http
            .get_with_query("/api/audit/entries", &filter)
            .await
    }

    /// Get audit statistics.
    pub async fn statistics(&self) -> ShiiooResult<AuditStatistics> {
        self.client.http.get("/api/audit/statistics").await
    }

    /// Verify the audit chain integrity.
    pub async fn verify_chain(&self) -> ShiiooResult<AuditChainVerification> {
        self.client.http.get("/api/audit/verify-chain").await
    }
}

/// Filter for audit entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuditFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<AuditCategory>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<DateTime<Utc>>,
}

/// Result of audit chain verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditChainVerification {
    pub is_valid: bool,
    pub message: String,
}
