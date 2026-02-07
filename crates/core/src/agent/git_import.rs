//! Git repository importer for discovering and importing skills.
//!
//! Clones git repositories, scans for `.md` files, and imports them
//! as skills (converting external formats as needed).

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use super::{
    format_converter::FormatConverterRegistry, skill_parser::SkillParser, Skill, SkillCategory,
};

/// A markdown file discovered in a git repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredFile {
    /// Path relative to the repo root.
    pub path: String,
    /// Title extracted from the first `#` heading, if any.
    pub title: Option<String>,
    /// First 200 characters of the file body.
    pub preview: String,
    /// File size in bytes.
    pub size: u64,
}

/// Result of scanning a git repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    /// Repository URL.
    pub repo_url: String,
    /// Git ref that was checked out.
    pub git_ref: String,
    /// Discovered markdown files.
    pub files: Vec<DiscoveredFile>,
}

/// Request to import specific files from a scanned repository.
#[derive(Debug, Clone, Deserialize)]
pub struct ImportRequest {
    /// Repository URL.
    pub repo_url: String,
    /// Git ref (branch/tag/commit).
    pub git_ref: Option<String>,
    /// Paths to import (from ScanResult).
    pub selected_paths: Vec<String>,
    /// Category to assign to imported skills.
    pub category: Option<SkillCategory>,
}

/// A successfully imported skill.
#[derive(Debug, Clone, Serialize)]
pub struct ImportedSkill {
    /// Path of the source file.
    pub source_path: String,
    /// Name of the imported skill.
    pub skill_name: String,
    /// How it was imported.
    pub method: ImportMethod,
}

/// How a file was converted into a skill.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportMethod {
    /// Parsed directly as SKILL.md.
    NativeParse,
    /// Converted from an external format.
    FormatConversion { format: String },
    /// Wrapped raw markdown as instructions.
    RawWrap,
}

/// An error encountered while importing a single file.
#[derive(Debug, Clone, Serialize)]
pub struct ImportError {
    /// Path of the file that failed.
    pub path: String,
    /// Error description.
    pub error: String,
}

/// Result of an import operation.
#[derive(Debug, Clone, Serialize)]
pub struct ImportResult {
    /// Successfully imported skills.
    pub imported: Vec<ImportedSkill>,
    /// Files that failed to import.
    pub errors: Vec<ImportError>,
}

/// Names of files to skip during scanning.
const SKIP_FILES: &[&str] = &[
    "README.md",
    "readme.md",
    "CHANGELOG.md",
    "changelog.md",
    "LICENSE.md",
    "license.md",
    "CONTRIBUTING.md",
    "contributing.md",
    "CODE_OF_CONDUCT.md",
];

/// Git repository importer.
pub struct GitImporter {
    /// Directory for caching cloned repos.
    cache_dir: PathBuf,
    /// Directory where imported skills are saved.
    local_skills_dir: PathBuf,
    /// Format converter registry.
    converter_registry: FormatConverterRegistry,
}

impl GitImporter {
    /// Create a new importer.
    pub fn new(cache_dir: PathBuf, local_skills_dir: PathBuf) -> Self {
        Self {
            cache_dir,
            local_skills_dir,
            converter_registry: FormatConverterRegistry::new(),
        }
    }

    /// Clone a git repository (shallow, depth 1).
    pub async fn clone_repo(&self, url: &str, git_ref: Option<&str>) -> Result<PathBuf> {
        let hash = Self::hash_url(url);
        let repo_dir = self.cache_dir.join("git").join(&hash);

        // If already cached, pull instead
        if repo_dir.exists() {
            tokio::fs::remove_dir_all(&repo_dir)
                .await
                .context("Failed to clean cached repo")?;
        }

        tokio::fs::create_dir_all(&repo_dir)
            .await
            .context("Failed to create cache directory")?;

        let mut cmd = tokio::process::Command::new("git");
        cmd.arg("clone").arg("--depth").arg("1");

        if let Some(r) = git_ref {
            cmd.arg("--branch").arg(r);
        }

        cmd.arg(url).arg(&repo_dir);

        let output = cmd
            .output()
            .await
            .context("Failed to execute git clone")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("git clone failed: {}", stderr));
        }

        Ok(repo_dir)
    }

    /// Scan a cloned repository for markdown files.
    pub fn scan_repo(&self, repo_path: &Path) -> Result<Vec<DiscoveredFile>> {
        let mut files = Vec::new();

        for entry in walkdir::WalkDir::new(repo_path)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();

            // Skip non-files
            if !path.is_file() {
                continue;
            }

            // Only .md files
            let ext = path.extension().and_then(|e| e.to_str());
            if ext != Some("md") {
                continue;
            }

            // Get relative path
            let relative = path
                .strip_prefix(repo_path)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();

            // Skip common non-skill files
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if SKIP_FILES.contains(&file_name) {
                continue;
            }

            // Skip hidden directories (e.g., .github/)
            if relative.starts_with('.') || relative.contains("/.") {
                continue;
            }

            // Read file for title and preview
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let title = Self::extract_title(&content);
            let preview = Self::extract_preview(&content, 200);
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);

            files.push(DiscoveredFile {
                path: relative,
                title,
                preview,
                size,
            });
        }

        // Sort by path for deterministic output
        files.sort_by(|a, b| a.path.cmp(&b.path));

        Ok(files)
    }

    /// Import selected files from a cloned repository.
    pub async fn import_files(&self, request: &ImportRequest) -> Result<ImportResult> {
        let hash = Self::hash_url(&request.repo_url);
        let repo_dir = self.cache_dir.join("git").join(&hash);

        if !repo_dir.exists() {
            return Err(anyhow!(
                "Repository not cached. Call clone_repo first."
            ));
        }

        let mut imported = Vec::new();
        let mut errors = Vec::new();

        for path in &request.selected_paths {
            let file_path = repo_dir.join(path);

            let content = match tokio::fs::read_to_string(&file_path).await {
                Ok(c) => c,
                Err(e) => {
                    errors.push(ImportError {
                        path: path.clone(),
                        error: format!("Failed to read file: {}", e),
                    });
                    continue;
                }
            };

            match self.import_single_file(path, &content, request.category.as_ref()) {
                Ok((skill, method)) => {
                    // Save to local skills directory
                    if let Err(e) = self.save_skill(&skill).await {
                        errors.push(ImportError {
                            path: path.clone(),
                            error: format!("Failed to save skill: {}", e),
                        });
                        continue;
                    }

                    imported.push(ImportedSkill {
                        source_path: path.clone(),
                        skill_name: skill.name,
                        method,
                    });
                }
                Err(e) => {
                    errors.push(ImportError {
                        path: path.clone(),
                        error: e.to_string(),
                    });
                }
            }
        }

        Ok(ImportResult { imported, errors })
    }

    /// Try to import a single file using multiple strategies.
    fn import_single_file(
        &self,
        path: &str,
        content: &str,
        category: Option<&SkillCategory>,
    ) -> Result<(Skill, ImportMethod)> {
        // Strategy 1: Try native SKILL.md parse
        if let Ok(mut skill) = SkillParser::parse(content) {
            if category.is_some() && skill.category.is_none() {
                skill.category = category.cloned();
            }
            return Ok((skill, ImportMethod::NativeParse));
        }

        // Strategy 2: Try format converters
        if let Ok(mut skill) = self.converter_registry.auto_detect_and_convert(content) {
            if category.is_some() && skill.category.is_none() {
                skill.category = category.cloned();
            }
            let format = skill
                .metadata
                .get("imported-from")
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());
            return Ok((skill, ImportMethod::FormatConversion { format }));
        }

        // Strategy 3: Raw wrap — use the file content as instructions
        let file_stem = Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("imported-skill");

        let name = SkillParser::sanitize_name(file_stem);
        if name.is_empty() {
            return Err(anyhow!("Could not derive a valid skill name from path: {}", path));
        }

        let title = Self::extract_title(content)
            .unwrap_or_else(|| format!("Imported from {}", path));

        let mut skill = Skill::new(name, title);
        skill.category = category.cloned();
        skill.instructions = content.to_string();
        skill
            .metadata
            .insert("imported-from".to_string(), "raw-wrap".to_string());
        skill
            .metadata
            .insert("source-path".to_string(), path.to_string());

        Ok((skill, ImportMethod::RawWrap))
    }

    /// Save a skill to the local skills directory as SKILL.md.
    async fn save_skill(&self, skill: &Skill) -> Result<()> {
        let skill_dir = self.local_skills_dir.join(&skill.name);
        tokio::fs::create_dir_all(&skill_dir)
            .await
            .context("Failed to create skill directory")?;

        let skill_md = Self::generate_skill_md(skill);
        let skill_path = skill_dir.join("SKILL.md");
        tokio::fs::write(&skill_path, skill_md)
            .await
            .context("Failed to write SKILL.md")?;

        Ok(())
    }

    /// Generate SKILL.md content from a Skill.
    fn generate_skill_md(skill: &Skill) -> String {
        let mut content = String::from("---\n");
        content.push_str(&format!("name: {}\n", skill.name));
        content.push_str(&format!(
            "description: \"{}\"\n",
            skill.description.replace('"', "\\\"")
        ));

        if let Some(ref category) = skill.category {
            content.push_str(&format!("category: {}\n", category));
        }
        if let Some(ref license) = skill.license {
            content.push_str(&format!("license: {}\n", license));
        }
        if !skill.allowed_tools.is_empty() {
            content.push_str(&format!(
                "allowed-tools: {}\n",
                skill.allowed_tools.join(" ")
            ));
        }
        if !skill.metadata.is_empty() {
            content.push_str("metadata:\n");
            let mut keys: Vec<_> = skill.metadata.keys().collect();
            keys.sort();
            for key in keys {
                content.push_str(&format!(
                    "  {}: \"{}\"\n",
                    key,
                    skill.metadata[key].replace('"', "\\\"")
                ));
            }
        }

        content.push_str("---\n\n");
        content.push_str(&skill.instructions);
        content.push('\n');

        content
    }

    /// Extract the first `#` heading from markdown content.
    fn extract_title(content: &str) -> Option<String> {
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some(title) = trimmed.strip_prefix("# ") {
                return Some(title.trim().to_string());
            }
        }
        None
    }

    /// Extract a preview of the body content (skipping frontmatter and headings).
    fn extract_preview(content: &str, max_chars: usize) -> String {
        let body = if content.trim_start().starts_with("---") {
            // Skip frontmatter
            match content[3..].find("\n---") {
                Some(pos) => &content[3 + pos + 4..],
                None => content,
            }
        } else {
            content
        };

        let preview: String = body
            .lines()
            .filter(|l| {
                let t = l.trim();
                !t.is_empty() && !t.starts_with('#')
            })
            .collect::<Vec<_>>()
            .join(" ");

        if preview.len() > max_chars {
            format!("{}...", &preview[..max_chars])
        } else {
            preview
        }
    }

    /// Hash a URL for use as a cache directory name.
    fn hash_url(url: &str) -> String {
        let hash = Sha256::digest(url.as_bytes());
        hex::encode(&hash[..8])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_extract_title() {
        assert_eq!(
            GitImporter::extract_title("# My Title\n\nContent"),
            Some("My Title".to_string())
        );
        assert_eq!(
            GitImporter::extract_title("No heading here"),
            None
        );
        assert_eq!(
            GitImporter::extract_title("## Not H1\n\nContent"),
            None
        );
    }

    #[test]
    fn test_extract_preview() {
        let content = "---\nname: test\n---\n\n# Heading\n\nSome body text here.\nMore text.";
        let preview = GitImporter::extract_preview(content, 100);
        assert!(preview.contains("Some body text here."));
        assert!(preview.contains("More text."));
        assert!(!preview.contains("Heading"));
    }

    #[test]
    fn test_hash_url() {
        let hash1 = GitImporter::hash_url("https://github.com/org/repo");
        let hash2 = GitImporter::hash_url("https://github.com/org/repo");
        let hash3 = GitImporter::hash_url("https://github.com/other/repo");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
        assert_eq!(hash1.len(), 16);
    }

    #[test]
    fn test_generate_skill_md() {
        let mut skill = Skill::new("test-skill", "A test skill for testing");
        skill.category = Some(SkillCategory::Testing);
        skill.allowed_tools = vec!["Read".to_string(), "Write".to_string()];
        skill.instructions = "# Test\n\nDo the thing.".to_string();

        let md = GitImporter::generate_skill_md(&skill);

        assert!(md.starts_with("---\n"));
        assert!(md.contains("name: test-skill"));
        assert!(md.contains("category: testing"));
        assert!(md.contains("allowed-tools: Read Write"));
        assert!(md.contains("# Test\n\nDo the thing."));
    }

    #[test]
    fn test_import_single_file_native() {
        let cache_dir = TempDir::new().unwrap();
        let skills_dir = TempDir::new().unwrap();
        let importer = GitImporter::new(
            cache_dir.path().to_path_buf(),
            skills_dir.path().to_path_buf(),
        );

        let content = r#"---
name: native-skill
description: A native SKILL.md file.
---

Instructions here.
"#;

        let (skill, method) = importer
            .import_single_file("native-skill/SKILL.md", content, None)
            .unwrap();

        assert_eq!(skill.name, "native-skill");
        assert!(matches!(method, ImportMethod::NativeParse));
    }

    #[test]
    fn test_import_single_file_contains_studio() {
        let cache_dir = TempDir::new().unwrap();
        let skills_dir = TempDir::new().unwrap();
        let importer = GitImporter::new(
            cache_dir.path().to_path_buf(),
            skills_dir.path().to_path_buf(),
        );

        let content = r##"---
name: Studio Agent
description: A contains-studio agent.
color: "#FF0000"
tools: "Read, Write, Bash"
---

You are an agent.
"##;

        let (skill, method) = importer
            .import_single_file("studio-agent.md", content, None)
            .unwrap();

        assert_eq!(skill.name, "studio-agent");
        assert!(matches!(
            method,
            ImportMethod::FormatConversion { format } if format == "contains-studio"
        ));
    }

    #[test]
    fn test_import_single_file_raw_wrap() {
        let cache_dir = TempDir::new().unwrap();
        let skills_dir = TempDir::new().unwrap();
        let importer = GitImporter::new(
            cache_dir.path().to_path_buf(),
            skills_dir.path().to_path_buf(),
        );

        let content = "# My Guide\n\nJust some markdown content without frontmatter.";

        let (skill, method) = importer
            .import_single_file("my-guide.md", content, Some(&SkillCategory::Documentation))
            .unwrap();

        assert_eq!(skill.name, "my-guide");
        assert_eq!(skill.category, Some(SkillCategory::Documentation));
        assert!(matches!(method, ImportMethod::RawWrap));
    }

    #[test]
    fn test_scan_repo() {
        let tmp = TempDir::new().unwrap();
        let repo_dir = tmp.path();

        // Create some test files
        std::fs::write(repo_dir.join("skill1.md"), "# Skill One\n\nContent here.").unwrap();
        std::fs::write(repo_dir.join("skill2.md"), "# Skill Two\n\nMore content.").unwrap();
        std::fs::write(repo_dir.join("README.md"), "# README\n\nShould be skipped.").unwrap();
        std::fs::create_dir_all(repo_dir.join("subdir")).unwrap();
        std::fs::write(
            repo_dir.join("subdir/nested.md"),
            "# Nested\n\nNested content.",
        )
        .unwrap();
        std::fs::write(repo_dir.join("not-markdown.txt"), "Skip this").unwrap();

        let cache_dir = TempDir::new().unwrap();
        let skills_dir = TempDir::new().unwrap();
        let importer = GitImporter::new(
            cache_dir.path().to_path_buf(),
            skills_dir.path().to_path_buf(),
        );

        let files = importer.scan_repo(repo_dir).unwrap();

        // Should find skill1.md, skill2.md, subdir/nested.md (not README.md, not .txt)
        assert_eq!(files.len(), 3);
        assert!(files.iter().any(|f| f.path == "skill1.md"));
        assert!(files.iter().any(|f| f.path == "skill2.md"));
        assert!(files.iter().any(|f| f.path == "subdir/nested.md"));

        // Check title extraction
        let skill1 = files.iter().find(|f| f.path == "skill1.md").unwrap();
        assert_eq!(skill1.title, Some("Skill One".to_string()));
    }
}
