//! Secrets API endpoints.

use crate::client::ShiiooClient;
use crate::error::ShiiooResult;
use serde::{Deserialize, Serialize};
use shiioo_core::secrets::{RotationPolicy, Secret, SecretId, SecretType, SecretVersion};
use std::collections::HashMap;

/// Secrets API for managing secrets.
pub struct SecretsApi<'a> {
    client: &'a ShiiooClient,
}

impl<'a> SecretsApi<'a> {
    pub(crate) fn new(client: &'a ShiiooClient) -> Self {
        Self { client }
    }

    /// List all secrets (without values).
    pub async fn list(&self) -> ShiiooResult<Vec<Secret>> {
        let response: ListSecretsResponse = self.client.http.get("/api/secrets").await?;
        Ok(response.secrets)
    }

    /// Get a specific secret (metadata only).
    pub async fn get(&self, secret_id: &SecretId) -> ShiiooResult<Secret> {
        self.client
            .http
            .get(&format!("/api/secrets/{}", secret_id.0))
            .await
    }

    /// Get the decrypted value of a secret.
    pub async fn get_value(&self, secret_id: &SecretId) -> ShiiooResult<String> {
        let response: SecretValueResponse = self
            .client
            .http
            .get(&format!("/api/secrets/{}/value", secret_id.0))
            .await?;
        Ok(response.value)
    }

    /// Create a new secret.
    pub async fn create(&self, request: CreateSecretRequest) -> ShiiooResult<Secret> {
        self.client.http.post("/api/secrets", &request).await
    }

    /// Update secret metadata.
    pub async fn update(
        &self,
        secret_id: &SecretId,
        request: UpdateSecretMetadataRequest,
    ) -> ShiiooResult<Secret> {
        self.client
            .http
            .put(&format!("/api/secrets/{}", secret_id.0), &request)
            .await
    }

    /// Delete a secret.
    pub async fn delete(&self, secret_id: &SecretId) -> ShiiooResult<DeleteSecretResponse> {
        self.client
            .http
            .delete(&format!("/api/secrets/{}", secret_id.0))
            .await
    }

    /// Rotate a secret (create new version).
    pub async fn rotate(
        &self,
        secret_id: &SecretId,
        new_value: &str,
    ) -> ShiiooResult<Secret> {
        let request = RotateSecretRequest {
            new_value: new_value.to_string(),
        };
        self.client
            .http
            .post(&format!("/api/secrets/{}/rotate", secret_id.0), &request)
            .await
    }

    /// Get version history for a secret.
    pub async fn versions(&self, secret_id: &SecretId) -> ShiiooResult<Vec<SecretVersion>> {
        let response: SecretVersionsResponse = self
            .client
            .http
            .get(&format!("/api/secrets/{}/versions", secret_id.0))
            .await?;
        Ok(response.versions)
    }

    /// Get secrets needing rotation.
    pub async fn needing_rotation(&self) -> ShiiooResult<Vec<Secret>> {
        let response: ListSecretsResponse =
            self.client.http.get("/api/secrets/rotation/needed").await?;
        Ok(response.secrets)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ListSecretsResponse {
    secrets: Vec<Secret>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SecretValueResponse {
    value: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct SecretVersionsResponse {
    versions: Vec<SecretVersion>,
}

/// Request to create a secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSecretRequest {
    pub name: String,
    pub description: String,
    pub secret_type: SecretType,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_policy: Option<RotationPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<HashMap<String, String>>,
}

/// Request to update secret metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSecretMetadataRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_policy: Option<RotationPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct RotateSecretRequest {
    new_value: String,
}

/// Response from deleting a secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteSecretResponse {
    pub message: String,
}
