//! Main client for the Shiioo SDK.

use crate::api::*;
use crate::config::{ClientConfig, RetryConfig};
use crate::error::{ShiiooError, ShiiooResult};
use crate::transport::{HttpTransport, WebSocketClient};
use std::sync::Arc;
use std::time::Duration;
use url::Url;

/// Main client for interacting with the Shiioo API.
#[derive(Clone)]
pub struct ShiiooClient {
    config: Arc<ClientConfig>,
    pub(crate) http: HttpTransport,
}

impl ShiiooClient {
    /// Create a new client builder.
    pub fn builder() -> ShiiooClientBuilder {
        ShiiooClientBuilder::new()
    }

    /// Create a client from configuration.
    fn from_config(config: ClientConfig) -> ShiiooResult<Self> {
        let config = Arc::new(config);
        let http = HttpTransport::new(config.clone())?;

        Ok(Self { config, http })
    }

    /// Get the health API.
    pub fn health(&self) -> HealthApi<'_> {
        HealthApi::new(self)
    }

    /// Get the runs API.
    pub fn runs(&self) -> RunsApi<'_> {
        RunsApi::new(self)
    }

    /// Get the jobs API.
    pub fn jobs(&self) -> JobsApi<'_> {
        JobsApi::new(self)
    }

    /// Get the roles API.
    pub fn roles(&self) -> RolesApi<'_> {
        RolesApi::new(self)
    }

    /// Get the policies API.
    pub fn policies(&self) -> PoliciesApi<'_> {
        PoliciesApi::new(self)
    }

    /// Get the organizations API.
    pub fn organizations(&self) -> OrganizationsApi<'_> {
        OrganizationsApi::new(self)
    }

    /// Get the templates API.
    pub fn templates(&self) -> TemplatesApi<'_> {
        TemplatesApi::new(self)
    }

    /// Get the capacity API.
    pub fn capacity(&self) -> CapacityApi<'_> {
        CapacityApi::new(self)
    }

    /// Get the routines API.
    pub fn routines(&self) -> RoutinesApi<'_> {
        RoutinesApi::new(self)
    }

    /// Get the approval boards API.
    pub fn approval_boards(&self) -> ApprovalBoardsApi<'_> {
        ApprovalBoardsApi::new(self)
    }

    /// Get the approvals API.
    pub fn approvals(&self) -> ApprovalsApi<'_> {
        ApprovalsApi::new(self)
    }

    /// Get the config changes API.
    pub fn config_changes(&self) -> ConfigChangesApi<'_> {
        ConfigChangesApi::new(self)
    }

    /// Get the metrics API.
    pub fn metrics(&self) -> MetricsApi<'_> {
        MetricsApi::new(self)
    }

    /// Get the analytics API.
    pub fn analytics(&self) -> AnalyticsApi<'_> {
        AnalyticsApi::new(self)
    }

    /// Get the secrets API.
    pub fn secrets(&self) -> SecretsApi<'_> {
        SecretsApi::new(self)
    }

    /// Get the tenants API.
    pub fn tenants(&self) -> TenantsApi<'_> {
        TenantsApi::new(self)
    }

    /// Get the cluster API.
    pub fn cluster(&self) -> ClusterApi<'_> {
        ClusterApi::new(self)
    }

    /// Get the audit API.
    pub fn audit(&self) -> AuditApi<'_> {
        AuditApi::new(self)
    }

    /// Get the RBAC API.
    pub fn rbac(&self) -> RbacApi<'_> {
        RbacApi::new(self)
    }

    /// Get the compliance API.
    pub fn compliance(&self) -> ComplianceApi<'_> {
        ComplianceApi::new(self)
    }

    /// Get the security API.
    pub fn security(&self) -> SecurityApi<'_> {
        SecurityApi::new(self)
    }

    /// Create a WebSocket subscription client.
    pub async fn subscribe(&self) -> ShiiooResult<WebSocketClient> {
        let mut ws = WebSocketClient::new(self.config.clone());
        ws.connect().await?;
        Ok(ws)
    }
}

/// Builder for creating a ShiiooClient.
pub struct ShiiooClientBuilder {
    base_url: Option<String>,
    api_key: Option<String>,
    timeout: Duration,
    retry_config: RetryConfig,
    tenant_id: Option<String>,
}

impl ShiiooClientBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            base_url: None,
            api_key: None,
            timeout: Duration::from_secs(30),
            retry_config: RetryConfig::default(),
            tenant_id: None,
        }
    }

    /// Set the base URL of the Shiioo server.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }

    /// Set the API key for authentication.
    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// Set the request timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the retry configuration.
    pub fn retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = config;
        self
    }

    /// Set the tenant ID for multi-tenant operations.
    pub fn tenant_id(mut self, id: impl Into<String>) -> Self {
        self.tenant_id = Some(id.into());
        self
    }

    /// Build the client.
    pub fn build(self) -> ShiiooResult<ShiiooClient> {
        let base_url_str = self
            .base_url
            .ok_or_else(|| ShiiooError::Config("base_url is required".to_string()))?;

        let base_url = Url::parse(&base_url_str)?;

        let config = ClientConfig {
            base_url,
            api_key: self.api_key,
            timeout: self.timeout,
            retry_config: self.retry_config,
            tenant_id: self.tenant_id,
        };

        ShiiooClient::from_config(config)
    }
}

impl Default for ShiiooClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}
