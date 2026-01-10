//! Templates API endpoints.

use crate::client::ShiiooClient;
use crate::error::ShiiooResult;
use serde::{Deserialize, Serialize};
use shiioo_core::types::{ProcessTemplate, TemplateId, TemplateInstance, WorkflowSpec};

/// Templates API for managing process templates.
pub struct TemplatesApi<'a> {
    client: &'a ShiiooClient,
}

impl<'a> TemplatesApi<'a> {
    pub(crate) fn new(client: &'a ShiiooClient) -> Self {
        Self { client }
    }

    /// List all templates.
    pub async fn list(&self) -> ShiiooResult<Vec<ProcessTemplate>> {
        let response: ListTemplatesResponse = self.client.http.get("/api/templates").await?;
        Ok(response.templates)
    }

    /// Get a specific template by ID.
    pub async fn get(&self, template_id: &TemplateId) -> ShiiooResult<ProcessTemplate> {
        self.client
            .http
            .get(&format!("/api/templates/{}", template_id.0))
            .await
    }

    /// Create or update a template.
    pub async fn create(&self, template: &ProcessTemplate) -> ShiiooResult<CreateTemplateResponse> {
        self.client.http.post("/api/templates", template).await
    }

    /// Delete a template.
    pub async fn delete(&self, template_id: &TemplateId) -> ShiiooResult<DeleteTemplateResponse> {
        self.client
            .http
            .delete(&format!("/api/templates/{}", template_id.0))
            .await
    }

    /// Instantiate a template with parameters.
    pub async fn instantiate(
        &self,
        template_id: &TemplateId,
        instance: &TemplateInstance,
    ) -> ShiiooResult<InstantiateTemplateResponse> {
        self.client
            .http
            .post(&format!("/api/templates/{}/instantiate", template_id.0), instance)
            .await
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ListTemplatesResponse {
    templates: Vec<ProcessTemplate>,
}

/// Response from creating a template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTemplateResponse {
    pub template_id: String,
    pub message: String,
}

/// Response from deleting a template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteTemplateResponse {
    pub message: String,
}

/// Response from instantiating a template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstantiateTemplateResponse {
    pub workflow: WorkflowSpec,
    pub message: String,
}
