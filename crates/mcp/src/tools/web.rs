// Web tools for fetching external content

use crate::protocol::{CallToolResult, ToolContent, ToolSchema};
use crate::tools::{json_schema_boolean, json_schema_object, json_schema_string, Tool, ToolTier};
use anyhow::{Context, Result};
use serde::Deserialize;
use url::Url;

/// Tool to fetch content from web URLs
pub struct WebFetchTool {
    allowed_domains: Vec<String>,
}

impl WebFetchTool {
    pub fn new() -> Self {
        Self {
            allowed_domains: vec![
                // Documentation sites
                "docs.rs".to_string(),
                "doc.rust-lang.org".to_string(),
                "developer.mozilla.org".to_string(),
                // Code hosting
                "github.com".to_string(),
                "raw.githubusercontent.com".to_string(),
                "gist.github.com".to_string(),
                // Package registries
                "crates.io".to_string(),
                "npmjs.com".to_string(),
                "pypi.org".to_string(),
                // General documentation
                "wikipedia.org".to_string(),
                "en.wikipedia.org".to_string(),
                // API documentation
                "api.github.com".to_string(),
            ],
        }
    }

    pub fn with_allowed_domains(domains: Vec<String>) -> Self {
        Self {
            allowed_domains: domains,
        }
    }

    fn is_domain_allowed(&self, url: &Url) -> bool {
        if let Some(host) = url.host_str() {
            // Check exact match
            if self.allowed_domains.contains(&host.to_string()) {
                return true;
            }

            // Check if it's a subdomain of an allowed domain
            for allowed in &self.allowed_domains {
                if host.ends_with(&format!(".{}", allowed)) || host == allowed {
                    return true;
                }
            }
        }
        false
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct WebFetchArgs {
    url: String,
    #[serde(default)]
    include_headers: bool,
}

#[async_trait::async_trait]
impl Tool for WebFetchTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "web_fetch".to_string(),
            description: format!(
                "Fetch content from a web URL. Only allowed domains: {}",
                self.allowed_domains.join(", ")
            ),
            input_schema: json_schema_object(
                serde_json::json!({
                    "url": json_schema_string("The URL to fetch"),
                    "include_headers": json_schema_boolean("Include HTTP response headers in output (default: false)")
                }),
                vec!["url"],
            ),
        }
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<CallToolResult> {
        let args: WebFetchArgs = serde_json::from_value(arguments)
            .context("Invalid arguments for web_fetch")?;

        // Parse and validate URL
        let url = match Url::parse(&args.url) {
            Ok(url) => url,
            Err(e) => {
                return Ok(CallToolResult {
                    content: vec![ToolContent::error(format!("Invalid URL: {}", e))],
                    is_error: Some(true),
                });
            }
        };

        // Check if domain is allowed
        if !self.is_domain_allowed(&url) {
            return Ok(CallToolResult {
                content: vec![ToolContent::error(format!(
                    "Domain not allowed: {}. Allowed domains: {}",
                    url.host_str().unwrap_or("unknown"),
                    self.allowed_domains.join(", ")
                ))],
                is_error: Some(true),
            });
        }

        // Only allow HTTP/HTTPS
        if url.scheme() != "http" && url.scheme() != "https" {
            return Ok(CallToolResult {
                content: vec![ToolContent::error(format!(
                    "Only HTTP/HTTPS URLs are supported, got: {}",
                    url.scheme()
                ))],
                is_error: Some(true),
            });
        }

        // Fetch the content
        let client = reqwest::Client::builder()
            .user_agent("shiioo-mcp/0.1.0")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        match client.get(url.as_str()).send().await {
            Ok(response) => {
                let status = response.status();
                let headers = response.headers().clone();

                match response.text().await {
                    Ok(body) => {
                        let mut output = String::new();

                        if args.include_headers {
                            output.push_str(&format!("HTTP Status: {}\n\n", status));
                            output.push_str("Headers:\n");
                            for (name, value) in headers.iter() {
                                output.push_str(&format!(
                                    "  {}: {}\n",
                                    name,
                                    value.to_str().unwrap_or("<non-utf8>")
                                ));
                            }
                            output.push_str("\nBody:\n");
                        }

                        output.push_str(&body);

                        // Truncate if too large (>100KB)
                        if output.len() > 100_000 {
                            output.truncate(100_000);
                            output.push_str("\n\n... (truncated, content too large)");
                        }

                        Ok(CallToolResult {
                            content: vec![ToolContent::text(output)],
                            is_error: None,
                        })
                    }
                    Err(e) => Ok(CallToolResult {
                        content: vec![ToolContent::error(format!(
                            "Failed to read response body: {}",
                            e
                        ))],
                        is_error: Some(true),
                    }),
                }
            }
            Err(e) => Ok(CallToolResult {
                content: vec![ToolContent::error(format!("HTTP request failed: {}", e))],
                is_error: Some(true),
            }),
        }
    }

    fn tier(&self) -> ToolTier {
        ToolTier::Tier0 // Read-only
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_domain_allowlist() {
        let tool = WebFetchTool::new();

        // Allowed domains
        assert!(tool.is_domain_allowed(&Url::parse("https://docs.rs/foo").unwrap()));
        assert!(tool.is_domain_allowed(&Url::parse("https://github.com/user/repo").unwrap()));
        assert!(tool.is_domain_allowed(&Url::parse("https://api.github.com/repos").unwrap()));

        // Subdomain of allowed domain
        assert!(tool.is_domain_allowed(&Url::parse("https://en.wikipedia.org").unwrap()));

        // Not allowed
        assert!(!tool.is_domain_allowed(&Url::parse("https://evil.com").unwrap()));
        assert!(!tool.is_domain_allowed(&Url::parse("https://example.com").unwrap()));
    }

    #[test]
    fn test_custom_allowlist() {
        let tool = WebFetchTool::with_allowed_domains(vec![
            "example.com".to_string(),
            "test.org".to_string(),
        ]);

        assert!(tool.is_domain_allowed(&Url::parse("https://example.com").unwrap()));
        assert!(tool.is_domain_allowed(&Url::parse("https://test.org/path").unwrap()));
        assert!(!tool.is_domain_allowed(&Url::parse("https://github.com").unwrap()));
    }
}
