//! Security API endpoints.

use crate::client::ShiiooClient;
use crate::error::ShiiooResult;
use shiioo_core::compliance::SecurityScanReport;

/// Security API for running security scans.
pub struct SecurityApi<'a> {
    client: &'a ShiiooClient,
}

impl<'a> SecurityApi<'a> {
    pub(crate) fn new(client: &'a ShiiooClient) -> Self {
        Self { client }
    }

    /// Run a security scan.
    pub async fn scan(&self) -> ShiiooResult<SecurityScanReport> {
        self.client.http.post("/api/security/scan", &()).await
    }
}
