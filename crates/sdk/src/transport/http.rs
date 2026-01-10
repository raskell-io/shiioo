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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RetryConfig;
    use serde::{Deserialize, Serialize};
    use std::time::Duration;
    use wiremock::matchers::{method, path, header};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct TestResponse {
        message: String,
        value: i32,
    }

    #[derive(Debug, Serialize)]
    struct TestRequest {
        name: String,
    }

    fn create_config(base_url: &str) -> Arc<ClientConfig> {
        Arc::new(ClientConfig {
            base_url: url::Url::parse(base_url).unwrap(),
            api_key: None,
            timeout: Duration::from_secs(30),
            retry_config: RetryConfig::no_retry(),
            tenant_id: None,
        })
    }

    fn create_config_with_auth(base_url: &str, api_key: &str) -> Arc<ClientConfig> {
        Arc::new(ClientConfig {
            base_url: url::Url::parse(base_url).unwrap(),
            api_key: Some(api_key.to_string()),
            timeout: Duration::from_secs(30),
            retry_config: RetryConfig::no_retry(),
            tenant_id: None,
        })
    }

    fn create_config_with_tenant(base_url: &str, tenant_id: &str) -> Arc<ClientConfig> {
        Arc::new(ClientConfig {
            base_url: url::Url::parse(base_url).unwrap(),
            api_key: None,
            timeout: Duration::from_secs(30),
            retry_config: RetryConfig::no_retry(),
            tenant_id: Some(tenant_id.to_string()),
        })
    }

    #[tokio::test]
    async fn test_get_request() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(TestResponse {
                message: "success".to_string(),
                value: 42,
            }))
            .mount(&server)
            .await;

        let config = create_config(&server.uri());
        let transport = HttpTransport::new(config).unwrap();

        let result: TestResponse = transport.get("/api/test").await.unwrap();
        assert_eq!(result.message, "success");
        assert_eq!(result.value, 42);
    }

    #[tokio::test]
    async fn test_post_request() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/create"))
            .respond_with(ResponseTemplate::new(200).set_body_json(TestResponse {
                message: "created".to_string(),
                value: 1,
            }))
            .mount(&server)
            .await;

        let config = create_config(&server.uri());
        let transport = HttpTransport::new(config).unwrap();

        let request = TestRequest {
            name: "test".to_string(),
        };
        let result: TestResponse = transport.post("/api/create", &request).await.unwrap();
        assert_eq!(result.message, "created");
    }

    #[tokio::test]
    async fn test_authorization_header() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/protected"))
            .and(header("Authorization", "Bearer sk-test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(TestResponse {
                message: "authorized".to_string(),
                value: 100,
            }))
            .mount(&server)
            .await;

        let config = create_config_with_auth(&server.uri(), "sk-test-key");
        let transport = HttpTransport::new(config).unwrap();

        let result: TestResponse = transport.get("/api/protected").await.unwrap();
        assert_eq!(result.message, "authorized");
    }

    #[tokio::test]
    async fn test_tenant_id_header() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/tenant"))
            .and(header("x-tenant-id", "tenant-abc"))
            .respond_with(ResponseTemplate::new(200).set_body_json(TestResponse {
                message: "tenant-scoped".to_string(),
                value: 200,
            }))
            .mount(&server)
            .await;

        let config = create_config_with_tenant(&server.uri(), "tenant-abc");
        let transport = HttpTransport::new(config).unwrap();

        let result: TestResponse = transport.get("/api/tenant").await.unwrap();
        assert_eq!(result.message, "tenant-scoped");
    }

    #[tokio::test]
    async fn test_error_on_400() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/bad"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(serde_json::json!({"error": "Bad Request"})),
            )
            .mount(&server)
            .await;

        let config = create_config(&server.uri());
        let transport = HttpTransport::new(config).unwrap();

        let result: ShiiooResult<TestResponse> = transport.get("/api/bad").await;
        assert!(result.is_err());
        match result {
            Err(ShiiooError::Api { status, .. }) => assert_eq!(status, 400),
            _ => panic!("Expected Api error"),
        }
    }

    #[tokio::test]
    async fn test_error_on_404() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/notfound"))
            .respond_with(ResponseTemplate::new(404).set_body_string("Not found"))
            .mount(&server)
            .await;

        let config = create_config(&server.uri());
        let transport = HttpTransport::new(config).unwrap();

        let result: ShiiooResult<TestResponse> = transport.get("/api/notfound").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_put_request() {
        let server = MockServer::start().await;

        Mock::given(method("PUT"))
            .and(path("/api/update"))
            .respond_with(ResponseTemplate::new(200).set_body_json(TestResponse {
                message: "updated".to_string(),
                value: 2,
            }))
            .mount(&server)
            .await;

        let config = create_config(&server.uri());
        let transport = HttpTransport::new(config).unwrap();

        let request = TestRequest {
            name: "updated".to_string(),
        };
        let result: TestResponse = transport.put("/api/update", &request).await.unwrap();
        assert_eq!(result.message, "updated");
    }

    #[tokio::test]
    async fn test_delete_request() {
        let server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/api/remove"))
            .respond_with(ResponseTemplate::new(200).set_body_json(TestResponse {
                message: "deleted".to_string(),
                value: 0,
            }))
            .mount(&server)
            .await;

        let config = create_config(&server.uri());
        let transport = HttpTransport::new(config).unwrap();

        let result: TestResponse = transport.delete("/api/remove").await.unwrap();
        assert_eq!(result.message, "deleted");
    }

    #[tokio::test]
    async fn test_build_url() {
        let config = create_config("http://localhost:8080");
        let transport = HttpTransport::new(config).unwrap();

        let url = transport.build_url("/api/test").unwrap();
        assert_eq!(url.as_str(), "http://localhost:8080/api/test");
    }

    #[tokio::test]
    async fn test_build_url_with_trailing_slash() {
        let config = create_config("http://localhost:8080/");
        let transport = HttpTransport::new(config).unwrap();

        let url = transport.build_url("api/test").unwrap();
        assert_eq!(url.as_str(), "http://localhost:8080/api/test");
    }
}
