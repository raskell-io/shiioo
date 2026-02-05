//! SKILL.md parser for the Agent Skills format.
//!
//! Parses skill files according to the [Agent Skills specification](https://agentskills.io/specification).
//!
//! A SKILL.md file consists of:
//! - YAML frontmatter (between `---` delimiters)
//! - Markdown body (instructions)

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

use super::{Skill, SkillAsset, SkillReference, SkillScript};

/// Raw frontmatter structure from SKILL.md
#[derive(Debug, Deserialize)]
struct SkillFrontmatter {
    /// Required: skill name (1-64 chars, lowercase alphanumeric + hyphens)
    name: String,

    /// Required: description of what the skill does
    description: String,

    /// Optional: license information
    #[serde(default)]
    license: Option<String>,

    /// Optional: environment requirements
    #[serde(default)]
    compatibility: Option<String>,

    /// Optional: pre-approved tools (space-delimited string)
    #[serde(default, rename = "allowed-tools")]
    allowed_tools: Option<String>,

    /// Optional: arbitrary metadata
    #[serde(default)]
    metadata: HashMap<String, String>,
}

/// Parser for SKILL.md files.
pub struct SkillParser;

impl SkillParser {
    /// Parse a SKILL.md file from its content.
    pub fn parse(content: &str) -> Result<Skill> {
        let (frontmatter, body) = Self::split_frontmatter(content)?;
        let fm: SkillFrontmatter =
            serde_yaml::from_str(&frontmatter).context("Failed to parse YAML frontmatter")?;

        // Validate name
        Self::validate_name(&fm.name)?;

        // Validate description
        if fm.description.is_empty() || fm.description.len() > 1024 {
            return Err(anyhow!(
                "description must be 1-1024 characters, got {}",
                fm.description.len()
            ));
        }

        // Parse allowed-tools (space-delimited)
        let allowed_tools = fm
            .allowed_tools
            .map(|s| s.split_whitespace().map(String::from).collect())
            .unwrap_or_default();

        Ok(Skill {
            name: fm.name,
            description: fm.description,
            license: fm.license,
            compatibility: fm.compatibility,
            allowed_tools,
            metadata: fm.metadata,
            instructions: body.trim().to_string(),
            scripts: Vec::new(),
            references: Vec::new(),
            assets: Vec::new(),
        })
    }

    /// Parse a SKILL.md file from a path.
    pub fn parse_file(path: &Path) -> Result<Skill> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read skill file: {}", path.display()))?;
        Self::parse(&content)
    }

    /// Parse a skill directory (SKILL.md + optional scripts/references/assets).
    pub fn parse_directory(dir: &Path) -> Result<Skill> {
        let skill_md_path = dir.join("SKILL.md");
        if !skill_md_path.exists() {
            return Err(anyhow!(
                "SKILL.md not found in directory: {}",
                dir.display()
            ));
        }

        let mut skill = Self::parse_file(&skill_md_path)?;

        // Validate that directory name matches skill name
        if let Some(dir_name) = dir.file_name().and_then(|n| n.to_str()) {
            if dir_name != skill.name {
                tracing::warn!(
                    "Skill directory name '{}' does not match skill name '{}' in SKILL.md",
                    dir_name,
                    skill.name
                );
            }
        }

        // Discover scripts
        let scripts_dir = dir.join("scripts");
        if scripts_dir.exists() {
            skill.scripts = Self::discover_scripts(&scripts_dir)?;
        }

        // Discover references
        let references_dir = dir.join("references");
        if references_dir.exists() {
            skill.references = Self::discover_references(&references_dir)?;
        }

        // Discover assets
        let assets_dir = dir.join("assets");
        if assets_dir.exists() {
            skill.assets = Self::discover_assets(&assets_dir)?;
        }

        Ok(skill)
    }

    /// Parse only the metadata from a SKILL.md file (for discovery).
    pub fn parse_metadata(content: &str) -> Result<super::SkillMetadata> {
        let (frontmatter, _) = Self::split_frontmatter(content)?;
        let fm: SkillFrontmatter =
            serde_yaml::from_str(&frontmatter).context("Failed to parse YAML frontmatter")?;

        let allowed_tools = fm
            .allowed_tools
            .map(|s| s.split_whitespace().map(String::from).collect())
            .unwrap_or_default();

        Ok(super::SkillMetadata {
            name: fm.name,
            description: fm.description,
            allowed_tools,
        })
    }

    /// Split content into frontmatter and body.
    fn split_frontmatter(content: &str) -> Result<(String, String)> {
        let content = content.trim();

        // Must start with ---
        if !content.starts_with("---") {
            return Err(anyhow!(
                "SKILL.md must start with YAML frontmatter (---)"
            ));
        }

        // Find the closing ---
        let after_first = &content[3..];
        let closing_pos = after_first
            .find("\n---")
            .ok_or_else(|| anyhow!("Missing closing --- for YAML frontmatter"))?;

        let frontmatter = after_first[..closing_pos].trim().to_string();
        let body = after_first[closing_pos + 4..].to_string();

        Ok((frontmatter, body))
    }

    /// Validate skill name according to spec.
    fn validate_name(name: &str) -> Result<()> {
        if name.is_empty() || name.len() > 64 {
            return Err(anyhow!("name must be 1-64 characters, got {}", name.len()));
        }

        if name.starts_with('-') || name.ends_with('-') {
            return Err(anyhow!("name must not start or end with hyphen"));
        }

        if name.contains("--") {
            return Err(anyhow!("name must not contain consecutive hyphens"));
        }

        // Must be lowercase alphanumeric + hyphens only
        for c in name.chars() {
            if !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '-' {
                return Err(anyhow!(
                    "name must contain only lowercase letters, numbers, and hyphens; found '{}'",
                    c
                ));
            }
        }

        Ok(())
    }

    /// Discover scripts in the scripts/ directory.
    fn discover_scripts(scripts_dir: &Path) -> Result<Vec<SkillScript>> {
        let mut scripts = Vec::new();

        for entry in walkdir::WalkDir::new(scripts_dir)
            .max_depth(2)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() {
                let path = entry.path();
                let relative_path = path
                    .strip_prefix(scripts_dir.parent().unwrap_or(scripts_dir))
                    .unwrap_or(path);

                let language = Self::detect_language(path);

                scripts.push(SkillScript {
                    path: relative_path.to_string_lossy().to_string(),
                    language,
                    description: None,
                });
            }
        }

        Ok(scripts)
    }

    /// Discover references in the references/ directory.
    fn discover_references(references_dir: &Path) -> Result<Vec<SkillReference>> {
        let mut references = Vec::new();

        for entry in walkdir::WalkDir::new(references_dir)
            .max_depth(2)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() {
                let path = entry.path();
                let relative_path = path
                    .strip_prefix(references_dir.parent().unwrap_or(references_dir))
                    .unwrap_or(path);

                // Try to extract title from markdown files
                let title = if path.extension().map(|e| e == "md").unwrap_or(false) {
                    Self::extract_markdown_title(path).ok()
                } else {
                    None
                };

                references.push(SkillReference {
                    path: relative_path.to_string_lossy().to_string(),
                    title,
                });
            }
        }

        Ok(references)
    }

    /// Discover assets in the assets/ directory.
    fn discover_assets(assets_dir: &Path) -> Result<Vec<SkillAsset>> {
        let mut assets = Vec::new();

        for entry in walkdir::WalkDir::new(assets_dir)
            .max_depth(2)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() {
                let path = entry.path();
                let relative_path = path
                    .strip_prefix(assets_dir.parent().unwrap_or(assets_dir))
                    .unwrap_or(path);

                let content_type = Self::detect_content_type(path);

                assets.push(SkillAsset {
                    path: relative_path.to_string_lossy().to_string(),
                    content_type,
                });
            }
        }

        Ok(assets)
    }

    /// Detect script language from file extension.
    fn detect_language(path: &Path) -> Option<String> {
        path.extension().and_then(|ext| {
            match ext.to_str()? {
                "py" => Some("python"),
                "sh" | "bash" => Some("bash"),
                "js" => Some("javascript"),
                "ts" => Some("typescript"),
                "rb" => Some("ruby"),
                "rs" => Some("rust"),
                "go" => Some("go"),
                _ => None,
            }
            .map(String::from)
        })
    }

    /// Detect content type from file extension.
    fn detect_content_type(path: &Path) -> Option<String> {
        path.extension().and_then(|ext| {
            match ext.to_str()? {
                "json" => Some("application/json"),
                "yaml" | "yml" => Some("application/yaml"),
                "xml" => Some("application/xml"),
                "html" => Some("text/html"),
                "css" => Some("text/css"),
                "txt" => Some("text/plain"),
                "md" => Some("text/markdown"),
                "png" => Some("image/png"),
                "jpg" | "jpeg" => Some("image/jpeg"),
                "gif" => Some("image/gif"),
                "svg" => Some("image/svg+xml"),
                "pdf" => Some("application/pdf"),
                _ => None,
            }
            .map(String::from)
        })
    }

    /// Extract title from a markdown file (first # heading).
    fn extract_markdown_title(path: &Path) -> Result<String> {
        let content = std::fs::read_to_string(path)?;
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some(title) = trimmed.strip_prefix("# ") {
                return Ok(title.trim().to_string());
            }
        }
        Err(anyhow!("No title found in markdown file"))
    }
}

/// Generate the XML representation for agent prompts.
pub fn skills_to_prompt_xml(skills: &[super::SkillMetadata]) -> String {
    let mut xml = String::from("<available_skills>\n");

    for skill in skills {
        xml.push_str("  <skill>\n");
        xml.push_str(&format!("    <name>{}</name>\n", escape_xml(&skill.name)));
        xml.push_str(&format!(
            "    <description>{}</description>\n",
            escape_xml(&skill.description)
        ));
        if !skill.allowed_tools.is_empty() {
            xml.push_str(&format!(
                "    <allowed-tools>{}</allowed-tools>\n",
                escape_xml(&skill.allowed_tools.join(" "))
            ));
        }
        xml.push_str("  </skill>\n");
    }

    xml.push_str("</available_skills>");
    xml
}

/// Generate XML with skill locations (for filesystem-based agents).
pub fn skills_to_prompt_xml_with_locations(
    skills: &[(super::SkillMetadata, String)],
) -> String {
    let mut xml = String::from("<available_skills>\n");

    for (skill, location) in skills {
        xml.push_str("  <skill>\n");
        xml.push_str(&format!("    <name>{}</name>\n", escape_xml(&skill.name)));
        xml.push_str(&format!(
            "    <description>{}</description>\n",
            escape_xml(&skill.description)
        ));
        xml.push_str(&format!(
            "    <location>{}</location>\n",
            escape_xml(location)
        ));
        xml.push_str("  </skill>\n");
    }

    xml.push_str("</available_skills>");
    xml
}

/// Escape XML special characters.
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_SKILL_MD: &str = r#"---
name: pdf-processing
description: Extract text and tables from PDF files, fill forms, merge documents.
license: Apache-2.0
metadata:
  author: example-org
  version: "1.0"
allowed-tools: Bash(pdftotext:*) Read Write
---

# PDF Processing

## When to use this skill

Use this skill when the user needs to work with PDF files.

## How to extract text

1. Use pdftotext for basic extraction
2. Use pdfplumber for tables

## Examples

```bash
pdftotext input.pdf output.txt
```
"#;

    #[test]
    fn test_parse_valid_skill() {
        let skill = SkillParser::parse(VALID_SKILL_MD).unwrap();

        assert_eq!(skill.name, "pdf-processing");
        assert_eq!(
            skill.description,
            "Extract text and tables from PDF files, fill forms, merge documents."
        );
        assert_eq!(skill.license, Some("Apache-2.0".to_string()));
        assert_eq!(skill.metadata.get("author"), Some(&"example-org".to_string()));
        assert_eq!(skill.metadata.get("version"), Some(&"1.0".to_string()));
        assert_eq!(skill.allowed_tools.len(), 3);
        assert!(skill.allowed_tools.contains(&"Bash(pdftotext:*)".to_string()));
        assert!(skill.instructions.contains("# PDF Processing"));
    }

    #[test]
    fn test_parse_minimal_skill() {
        let content = r#"---
name: minimal-skill
description: A minimal skill for testing.
---

Just some instructions.
"#;

        let skill = SkillParser::parse(content).unwrap();
        assert_eq!(skill.name, "minimal-skill");
        assert_eq!(skill.description, "A minimal skill for testing.");
        assert!(skill.license.is_none());
        assert!(skill.allowed_tools.is_empty());
    }

    #[test]
    fn test_parse_metadata_only() {
        let metadata = SkillParser::parse_metadata(VALID_SKILL_MD).unwrap();

        assert_eq!(metadata.name, "pdf-processing");
        assert_eq!(
            metadata.description,
            "Extract text and tables from PDF files, fill forms, merge documents."
        );
        assert_eq!(metadata.allowed_tools.len(), 3);
    }

    #[test]
    fn test_invalid_name_uppercase() {
        let content = r#"---
name: PDF-Processing
description: Test skill
---

Instructions
"#;

        let result = SkillParser::parse(content);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("lowercase"));
    }

    #[test]
    fn test_invalid_name_starts_with_hyphen() {
        let content = r#"---
name: -invalid
description: Test skill
---

Instructions
"#;

        let result = SkillParser::parse(content);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("start or end with hyphen"));
    }

    #[test]
    fn test_invalid_name_consecutive_hyphens() {
        let content = r#"---
name: invalid--name
description: Test skill
---

Instructions
"#;

        let result = SkillParser::parse(content);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("consecutive hyphens"));
    }

    #[test]
    fn test_missing_frontmatter() {
        let content = "# Just Markdown\n\nNo frontmatter here.";

        let result = SkillParser::parse(content);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must start with YAML frontmatter"));
    }

    #[test]
    fn test_missing_closing_frontmatter() {
        let content = r#"---
name: test
description: Test
No closing delimiter
"#;

        let result = SkillParser::parse(content);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Missing closing ---"));
    }

    #[test]
    fn test_validate_name() {
        // Valid names
        assert!(SkillParser::validate_name("pdf-processing").is_ok());
        assert!(SkillParser::validate_name("skill123").is_ok());
        assert!(SkillParser::validate_name("a").is_ok());
        assert!(SkillParser::validate_name("my-awesome-skill").is_ok());

        // Invalid names
        assert!(SkillParser::validate_name("").is_err());
        assert!(SkillParser::validate_name("PDF").is_err());
        assert!(SkillParser::validate_name("-start").is_err());
        assert!(SkillParser::validate_name("end-").is_err());
        assert!(SkillParser::validate_name("double--hyphen").is_err());
        assert!(SkillParser::validate_name("under_score").is_err());
    }

    #[test]
    fn test_detect_language() {
        assert_eq!(
            SkillParser::detect_language(Path::new("script.py")),
            Some("python".to_string())
        );
        assert_eq!(
            SkillParser::detect_language(Path::new("script.sh")),
            Some("bash".to_string())
        );
        assert_eq!(
            SkillParser::detect_language(Path::new("script.js")),
            Some("javascript".to_string())
        );
        assert_eq!(
            SkillParser::detect_language(Path::new("script.unknown")),
            None
        );
    }

    #[test]
    fn test_detect_content_type() {
        assert_eq!(
            SkillParser::detect_content_type(Path::new("data.json")),
            Some("application/json".to_string())
        );
        assert_eq!(
            SkillParser::detect_content_type(Path::new("image.png")),
            Some("image/png".to_string())
        );
        assert_eq!(
            SkillParser::detect_content_type(Path::new("doc.pdf")),
            Some("application/pdf".to_string())
        );
    }

    #[test]
    fn test_skills_to_prompt_xml() {
        let skills = vec![
            super::super::SkillMetadata {
                name: "skill-one".to_string(),
                description: "First skill".to_string(),
                allowed_tools: vec!["tool1".to_string()],
            },
            super::super::SkillMetadata {
                name: "skill-two".to_string(),
                description: "Second skill".to_string(),
                allowed_tools: vec![],
            },
        ];

        let xml = skills_to_prompt_xml(&skills);

        assert!(xml.contains("<available_skills>"));
        assert!(xml.contains("<name>skill-one</name>"));
        assert!(xml.contains("<description>First skill</description>"));
        assert!(xml.contains("<allowed-tools>tool1</allowed-tools>"));
        assert!(xml.contains("<name>skill-two</name>"));
        assert!(xml.contains("</available_skills>"));
    }

    #[test]
    fn test_escape_xml() {
        assert_eq!(escape_xml("test"), "test");
        assert_eq!(escape_xml("<script>"), "&lt;script&gt;");
        assert_eq!(escape_xml("a & b"), "a &amp; b");
        assert_eq!(escape_xml("\"quoted\""), "&quot;quoted&quot;");
    }
}
