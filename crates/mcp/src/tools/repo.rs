// Repository tools for reading files and directories

use crate::protocol::{CallToolResult, ToolContent, ToolSchema};
use crate::tools::{json_schema_object, json_schema_string, Tool, ToolTier};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Tool to read files from the repository
pub struct RepoReadTool {
    base_path: PathBuf,
}

impl RepoReadTool {
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }

    /// Check if a path is safe to read (not a secret file)
    fn is_safe_path(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy().to_lowercase();

        // Deny list of secret patterns
        let deny_patterns = [
            ".env",
            "credentials",
            "secrets",
            "private",
            "id_rsa",
            "id_ed25519",
            ".pem",
            ".key",
            "password",
            "token",
            "api_key",
            "aws",
            "gcp",
            ".git/config",
        ];

        for pattern in &deny_patterns {
            if path_str.contains(pattern) {
                return false;
            }
        }

        // Must be within base_path
        if let Ok(canonical) = path.canonicalize() {
            if let Ok(base_canonical) = self.base_path.canonicalize() {
                return canonical.starts_with(base_canonical);
            }
        }

        true
    }
}

#[derive(Debug, Deserialize)]
struct RepoReadArgs {
    path: String,
}

#[async_trait::async_trait]
impl Tool for RepoReadTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "repo_read".to_string(),
            description: "Read a file from the repository. Cannot read secret files (.env, credentials, private keys, etc.)".to_string(),
            input_schema: json_schema_object(
                serde_json::json!({
                    "path": json_schema_string("Path to the file to read (relative to repository root)")
                }),
                vec!["path"],
            ),
        }
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<CallToolResult> {
        let args: RepoReadArgs = serde_json::from_value(arguments)
            .context("Invalid arguments for repo_read")?;

        let path = self.base_path.join(&args.path);

        // Security check
        if !self.is_safe_path(&path) {
            return Ok(CallToolResult {
                content: vec![ToolContent::error(format!(
                    "Access denied: {} appears to be a secret file or outside repository",
                    args.path
                ))],
                is_error: Some(true),
            });
        }

        // Check if file exists
        if !path.exists() {
            return Ok(CallToolResult {
                content: vec![ToolContent::error(format!("File not found: {}", args.path))],
                is_error: Some(true),
            });
        }

        // Check if it's a directory
        if path.is_dir() {
            // List directory contents
            let mut entries = Vec::new();
            match std::fs::read_dir(&path) {
                Ok(dir) => {
                    for entry in dir {
                        if let Ok(entry) = entry {
                            let name = entry.file_name().to_string_lossy().to_string();
                            let is_dir = entry.path().is_dir();
                            entries.push(format!("{}{}", name, if is_dir { "/" } else { "" }));
                        }
                    }
                }
                Err(e) => {
                    return Ok(CallToolResult {
                        content: vec![ToolContent::error(format!(
                            "Failed to read directory: {}",
                            e
                        ))],
                        is_error: Some(true),
                    });
                }
            }

            entries.sort();
            return Ok(CallToolResult {
                content: vec![ToolContent::text(format!(
                    "Directory: {}\n\nContents ({} items):\n{}",
                    args.path,
                    entries.len(),
                    entries.join("\n")
                ))],
                is_error: None,
            });
        }

        // Read file contents
        match tokio::fs::read_to_string(&path).await {
            Ok(content) => {
                let line_count = content.lines().count();
                Ok(CallToolResult {
                    content: vec![ToolContent::text(format!(
                        "File: {} ({} lines)\n\n{}",
                        args.path, line_count, content
                    ))],
                    is_error: None,
                })
            }
            Err(e) => {
                // Try reading as binary if UTF-8 fails
                match tokio::fs::read(&path).await {
                    Ok(bytes) => Ok(CallToolResult {
                        content: vec![ToolContent::text(format!(
                            "File: {} (binary, {} bytes)\n\nBinary files cannot be displayed as text. Use appropriate tools to view binary content.",
                            args.path,
                            bytes.len()
                        ))],
                        is_error: None,
                    }),
                    Err(_) => Ok(CallToolResult {
                        content: vec![ToolContent::error(format!("Failed to read file: {}", e))],
                        is_error: Some(true),
                    }),
                }
            }
        }
    }

    fn tier(&self) -> ToolTier {
        ToolTier::Tier0 // Read-only
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_repo_read_safe_paths() {
        let temp_dir = TempDir::new().unwrap();
        let tool = RepoReadTool::new(temp_dir.path().to_path_buf());

        // Test safe paths
        assert!(tool.is_safe_path(&temp_dir.path().join("README.md")));
        assert!(tool.is_safe_path(&temp_dir.path().join("src/main.rs")));

        // Test unsafe paths
        assert!(!tool.is_safe_path(&temp_dir.path().join(".env")));
        assert!(!tool.is_safe_path(&temp_dir.path().join("credentials.json")));
        assert!(!tool.is_safe_path(&temp_dir.path().join("secret.key")));
        assert!(!tool.is_safe_path(&temp_dir.path().join(".git/config")));
    }

    #[tokio::test]
    async fn test_repo_read_file() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.txt");
        std::fs::write(&test_file, "Hello, world!").unwrap();

        let tool = RepoReadTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(serde_json::json!({"path": "test.txt"}))
            .await
            .unwrap();

        assert!(result.is_error.is_none());
        assert_eq!(result.content.len(), 1);
    }

    #[tokio::test]
    async fn test_repo_read_directory() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::create_dir(temp_dir.path().join("subdir")).unwrap();
        std::fs::write(temp_dir.path().join("file.txt"), "test").unwrap();

        let tool = RepoReadTool::new(temp_dir.path().to_path_buf());
        let result = tool
            .execute(serde_json::json!({"path": "."}))
            .await
            .unwrap();

        assert!(result.is_error.is_none());
    }
}
