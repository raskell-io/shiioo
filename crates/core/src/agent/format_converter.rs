//! Format converters for transforming external skill formats into SKILL.md.
//!
//! Supports auto-detection of formats and conversion to the Agent Skills spec.

use anyhow::{anyhow, Result};

use super::{skill_parser::SkillParser, Skill};

/// Trait for converting external formats into a Skill.
pub trait FormatConverter: Send + Sync {
    /// Name of the format this converter handles.
    fn format_name(&self) -> &str;

    /// Check if the content matches this format.
    fn can_convert(&self, content: &str) -> bool;

    /// Convert the content into a Skill.
    fn convert(&self, content: &str) -> Result<Skill>;
}

/// Converter for contains-studio agent files.
///
/// Contains-studio agents use YAML frontmatter with fields like:
/// `name`, `description`, `color`, `tools` (comma-separated).
pub struct ContainsStudioConverter;

impl ContainsStudioConverter {
    /// Maximum description length before truncation.
    const MAX_DESCRIPTION_LEN: usize = 1024;

    /// Truncate a string at a sentence boundary within the given limit.
    fn truncate_at_sentence(s: &str, max_len: usize) -> String {
        if s.len() <= max_len {
            return s.to_string();
        }

        let truncated = &s[..max_len];

        // Try to find a sentence boundary
        for (i, _) in truncated.rmatch_indices(|c: char| c == '.' || c == '!' || c == '?') {
            if i > 0 {
                return s[..=i].to_string();
            }
        }

        // No sentence boundary found, truncate at word boundary
        if let Some(last_space) = truncated.rfind(' ') {
            format!("{}...", &s[..last_space])
        } else {
            format!("{}...", truncated)
        }
    }
}

impl FormatConverter for ContainsStudioConverter {
    fn format_name(&self) -> &str {
        "contains-studio"
    }

    fn can_convert(&self, content: &str) -> bool {
        let content = content.trim();
        if !content.starts_with("---") {
            return false;
        }

        // Look for contains-studio signature fields in frontmatter
        let after_first = &content[3..];
        let closing = match after_first.find("\n---") {
            Some(pos) => pos,
            None => return false,
        };

        let frontmatter = &after_first[..closing];
        let has_color = frontmatter
            .lines()
            .any(|l| l.trim_start().starts_with("color:"));
        let has_tools = frontmatter.lines().any(|l| {
            let trimmed = l.trim_start();
            trimmed.starts_with("tools:") && trimmed.contains(',')
        });

        has_color && has_tools
    }

    fn convert(&self, content: &str) -> Result<Skill> {
        let (frontmatter_str, body) = SkillParser::split_frontmatter(content)?;

        // Parse YAML manually since the format differs from SKILL.md
        let yaml: serde_yaml::Value =
            serde_yaml::from_str(&frontmatter_str).map_err(|e| anyhow!("Invalid YAML: {}", e))?;

        let mapping = yaml
            .as_mapping()
            .ok_or_else(|| anyhow!("Frontmatter must be a YAML mapping"))?;

        // Extract name
        let raw_name = mapping
            .get(serde_yaml::Value::String("name".to_string()))
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'name' field"))?;

        let name = SkillParser::sanitize_name(raw_name);
        if name.is_empty() {
            return Err(anyhow!("Could not derive valid skill name from '{}'", raw_name));
        }

        // Extract description
        let description = mapping
            .get(serde_yaml::Value::String("description".to_string()))
            .and_then(|v| v.as_str())
            .unwrap_or("Imported skill");

        let description =
            Self::truncate_at_sentence(description, Self::MAX_DESCRIPTION_LEN);

        // Extract tools (comma-separated)
        let allowed_tools: Vec<String> = mapping
            .get(serde_yaml::Value::String("tools".to_string()))
            .and_then(|v| v.as_str())
            .map(|s| {
                s.split(',')
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        // Extract color (preserve as metadata)
        let mut metadata = std::collections::HashMap::new();
        if let Some(color) = mapping
            .get(serde_yaml::Value::String("color".to_string()))
            .and_then(|v| v.as_str())
        {
            metadata.insert("original-color".to_string(), color.to_string());
        }
        metadata.insert(
            "imported-from".to_string(),
            "contains-studio".to_string(),
        );

        Ok(Skill {
            name,
            description,
            category: None,
            license: None,
            compatibility: None,
            allowed_tools,
            metadata,
            instructions: body.trim().to_string(),
            scripts: Vec::new(),
            references: Vec::new(),
            assets: Vec::new(),
        })
    }
}

/// Registry of format converters for auto-detection.
pub struct FormatConverterRegistry {
    converters: Vec<Box<dyn FormatConverter>>,
}

impl FormatConverterRegistry {
    /// Create a new registry with the default set of converters.
    pub fn new() -> Self {
        Self {
            converters: vec![Box::new(ContainsStudioConverter)],
        }
    }

    /// Add a converter to the registry.
    pub fn register(&mut self, converter: Box<dyn FormatConverter>) {
        self.converters.push(converter);
    }

    /// Try each converter in order and return the first successful conversion.
    pub fn auto_detect_and_convert(&self, content: &str) -> Result<Skill> {
        for converter in &self.converters {
            if converter.can_convert(content) {
                return converter.convert(content);
            }
        }
        Err(anyhow!(
            "No format converter recognized this content. \
             Supported formats: {}",
            self.converters
                .iter()
                .map(|c| c.format_name())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }

    /// Convert content using a specific named format.
    pub fn convert_with(&self, format_name: &str, content: &str) -> Result<Skill> {
        let converter = self
            .converters
            .iter()
            .find(|c| c.format_name() == format_name)
            .ok_or_else(|| anyhow!("Unknown format: {}", format_name))?;

        converter.convert(content)
    }

    /// List available format names.
    pub fn available_formats(&self) -> Vec<&str> {
        self.converters.iter().map(|c| c.format_name()).collect()
    }
}

impl Default for FormatConverterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTAINS_STUDIO_AGENT: &str = r##"---
name: Code Reviewer Pro
description: An expert code reviewer that checks for bugs, security issues, and best practices.
color: "#4A90D9"
tools: "Write, Read, MultiEdit, Bash"
---

# Code Reviewer Pro

You are an expert code reviewer. When reviewing code:

1. Check for bugs and logic errors
2. Look for security vulnerabilities
3. Suggest improvements for readability
"##;

    #[test]
    fn test_contains_studio_detection() {
        let converter = ContainsStudioConverter;
        assert!(converter.can_convert(CONTAINS_STUDIO_AGENT));

        // Should not match standard SKILL.md
        let standard = r#"---
name: my-skill
description: A standard skill.
allowed-tools: Read Write
---

Instructions.
"#;
        assert!(!converter.can_convert(standard));
    }

    #[test]
    fn test_contains_studio_conversion() {
        let converter = ContainsStudioConverter;
        let skill = converter.convert(CONTAINS_STUDIO_AGENT).unwrap();

        assert_eq!(skill.name, "code-reviewer-pro");
        assert!(skill.description.contains("expert code reviewer"));
        assert_eq!(skill.allowed_tools.len(), 4);
        assert!(skill.allowed_tools.contains(&"Write".to_string()));
        assert!(skill.allowed_tools.contains(&"Read".to_string()));
        assert!(skill.allowed_tools.contains(&"MultiEdit".to_string()));
        assert!(skill.allowed_tools.contains(&"Bash".to_string()));
        assert_eq!(
            skill.metadata.get("original-color"),
            Some(&"#4A90D9".to_string())
        );
        assert_eq!(
            skill.metadata.get("imported-from"),
            Some(&"contains-studio".to_string())
        );
        assert!(skill.instructions.contains("expert code reviewer"));
    }

    #[test]
    fn test_auto_detect_and_convert() {
        let registry = FormatConverterRegistry::new();
        let skill = registry
            .auto_detect_and_convert(CONTAINS_STUDIO_AGENT)
            .unwrap();
        assert_eq!(skill.name, "code-reviewer-pro");
    }

    #[test]
    fn test_auto_detect_unknown_format() {
        let registry = FormatConverterRegistry::new();
        let result = registry.auto_detect_and_convert("Just some plain text");
        assert!(result.is_err());
    }

    #[test]
    fn test_truncate_at_sentence() {
        let short = "Short description.";
        assert_eq!(
            ContainsStudioConverter::truncate_at_sentence(short, 100),
            short
        );

        let long = "First sentence. Second sentence. Third sentence that is very long.";
        let truncated = ContainsStudioConverter::truncate_at_sentence(long, 35);
        assert!(truncated.len() <= 35);
        assert!(truncated.ends_with('.'));
    }

    #[test]
    fn test_convert_with_named_format() {
        let registry = FormatConverterRegistry::new();
        let skill = registry
            .convert_with("contains-studio", CONTAINS_STUDIO_AGENT)
            .unwrap();
        assert_eq!(skill.name, "code-reviewer-pro");

        let result = registry.convert_with("nonexistent", CONTAINS_STUDIO_AGENT);
        assert!(result.is_err());
    }

    #[test]
    fn test_available_formats() {
        let registry = FormatConverterRegistry::new();
        let formats = registry.available_formats();
        assert!(formats.contains(&"contains-studio"));
    }

    #[test]
    fn test_missing_name_field() {
        let content = r##"---
description: No name here.
color: "#FF0000"
tools: "Read, Write"
---

Body.
"##;
        let converter = ContainsStudioConverter;
        let result = converter.convert(content);
        assert!(result.is_err());
    }
}
