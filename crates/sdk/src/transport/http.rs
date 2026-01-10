//! HTTP transport layer for the Shiioo SDK.

use crate::config::ClientConfig;
use crate::error::{ShiiooError, ShiiooResult};
use reqwest::{header, Client, RequestBuilder, Response};
use serde::{de::DeserializeOwned, Serialize};
use std::sync::Arc;
use tracing::{debug, warn};

/// HTTP transport for making API requests.
#[derive(Debug, Clone)]
pub struct HttpTransport {
    client: Client,
    config: Arc<ClientConfig>,
}

impl HttpTransport {
    /// Create a new HTTP transport with the given configuration.
    pub fn new(config: Arc<ClientConfig>) -> ShiiooResult<Self> {
        let mut headers = header::HeaderMap::new();

        // Add API key header if present
        if let Some(ref api_key) = config.api_key {
            headers.insert(
                header::AUTHORIZATION,
                header::HeaderValue::from_str(&format!("Bearer {}", api_key))
                    .map_err(|_| ShiiooError::Config("Invalid API key format".to_string()))?,
            );
        }

        // Add tenant ID header if present
        if let Some(ref tenant_id) = config.tenant_id {
            headers.insert(
                header::HeaderName::from_static("x-tenant-id"),
                header::HeaderValue::from_str(tenant_id)
                    .map_err(|_| ShiiooError::Config("Invalid tenant ID format".to_string()))?,
            );
        }

        let client = Client::builder()
            .timeout(config.timeout)
            .default_headers(headers)
            .build()?;

        Ok(Self { client, config })
    }

    /// Build a URL for the given path.
    fn build_url(&self, path: &str) -> ShiiooResult<url::Url> {
        self.config
            .base_url
            .join(path)
            .map_err(|e| ShiiooError::InvalidUrl(e))
    }

    /// Execute a request with retries.
    async fn execute_with_retry(&self, request_builder: RequestBuilder) -> ShiiooResult<Response> {
        let retry_config = &self.config.retry_config;
        let mut attempts = 0;

        loop {
            let request = request_builder
                .try_clone()
                .ok_or_else(|| ShiiooError::Config("Request cannot be cloned".to_string()))?;

            match request.send().await {
                Ok(response) => {
                    let status = response.status().as_u16();

                    if response.status().is_success() {
                        return Ok(response);
                    }

                    // Check if we should retry
                    if attempts < retry_config.max_retries
                        && retry_config.should_retry_status(status)
                    {
                        let backoff = retry_config.backoff_for_attempt(attempts);
                        warn!(
                            status = status,
                            attempt = attempts + 1,
                            backoff_ms = backoff.as_millis(),
                            "Request failed, retrying"
                        );
                        tokio::time::sleep(backoff).await;
                        attempts += 1;
                        continue;
                    }

                    // Return error for non-success status
                    let body = response.text().await.unwrap_or_default();
                    return Err(ShiiooError::from_response(status, &body));
                }
                Err(e) => {
                    if attempts < retry_config.max_retries && e.is_timeout() {
                        let backoff = retry_config.backoff_for_attempt(attempts);
                        warn!(
                            attempt = attempts + 1,
                            backoff_ms = backoff.as_millis(),
                            "Request timed out, retrying"
                        );
                        tokio::time::sleep(backoff).await;
                        attempts += 1;
                        continue;
                    }
                    return Err(e.into());
                }
            }
        }
    }

    /// Execute a GET request.
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> ShiiooResult<T> {
        let url = self.build_url(path)?;
        debug!(url = %url, "GET request");

        let response = self.execute_with_retry(self.client.get(url)).await?;
        let body = response.json().await?;
        Ok(body)
    }

    /// Execute a GET request with query parameters.
    pub async fn get_with_query<T: DeserializeOwned, Q: Serialize>(
        &self,
        path: &str,
        query: &Q,
    ) -> ShiiooResult<T> {
        let url = self.build_url(path)?;
        debug!(url = %url, "GET request with query");

        let response = self
            .execute_with_retry(self.client.get(url).query(query))
            .await?;
        let body = response.json().await?;
        Ok(body)
    }

    /// Execute a POST request.
    pub async fn post<T: DeserializeOwned, B: Serialize>(&self, path: &str, body: &B) -> ShiiooResult<T> {
        let url = self.build_url(path)?;
        debug!(url = %url, "POST request");

        let response = self
            .execute_with_retry(self.client.post(url).json(body))
            .await?;
        let body = response.json().await?;
        Ok(body)
    }

    /// Execute a POST request without a response body.
    pub async fn post_no_response<B: Serialize>(&self, path: &str, body: &B) -> ShiiooResult<()> {
        let url = self.build_url(path)?;
        debug!(url = %url, "POST request (no response)");

        self.execute_with_retry(self.client.post(url).json(body))
            .await?;
        Ok(())
    }

    /// Execute a PUT request.
    pub async fn put<T: DeserializeOwned, B: Serialize>(&self, path: &str, body: &B) -> ShiiooResult<T> {
        let url = self.build_url(path)?;
        debug!(url = %url, "PUT request");

        let response = self
            .execute_with_retry(self.client.put(url).json(body))
            .await?;
        let body = response.json().await?;
        Ok(body)
    }

    /// Execute a DELETE request.
    pub async fn delete<T: DeserializeOwned>(&self, path: &str) -> ShiiooResult<T> {
        let url = self.build_url(path)?;
        debug!(url = %url, "DELETE request");

        let response = self.execute_with_retry(self.client.delete(url)).await?;
        let body = response.json().await?;
        Ok(body)
    }

    /// Execute a DELETE request without a response body.
    pub async fn delete_no_response(&self, path: &str) -> ShiiooResult<()> {
        let url = self.build_url(path)?;
        debug!(url = %url, "DELETE request (no response)");

        self.execute_with_retry(self.client.delete(url)).await?;
        Ok(())
    }
}
