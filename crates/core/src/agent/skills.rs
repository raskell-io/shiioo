//! Agent skills based on the Agent Skills format.
//!
//! Skills define what an agent knows and how to perform specific tasks.
//! We adopt the [Agent Skills](https://agentskills.io) format directly.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

/// Skill category for organizing and browsing skills.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillCategory {
    Engineering,
    Design,
    Marketing,
    Product,
    Operations,
    Testing,
    Security,
    DataAnalysis,
    Documentation,
    IncidentResponse,
    Custom(String),
}

impl fmt::Display for SkillCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Engineering => write!(f, "engineering"),
            Self::Design => write!(f, "design"),
            Self::Marketing => write!(f, "marketing"),
            Self::Product => write!(f, "product"),
            Self::Operations => write!(f, "operations"),
            Self::Testing => write!(f, "testing"),
            Self::Security => write!(f, "security"),
            Self::DataAnalysis => write!(f, "data_analysis"),
            Self::Documentation => write!(f, "documentation"),
            Self::IncidentResponse => write!(f, "incident_response"),
            Self::Custom(name) => write!(f, "{}", name),
        }
    }
}

impl SkillCategory {
    /// Parse a category from a string.
    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().replace('-', "_").as_str() {
            "engineering" => Self::Engineering,
            "design" => Self::Design,
            "marketing" => Self::Marketing,
            "product" => Self::Product,
            "operations" => Self::Operations,
            "testing" => Self::Testing,
            "security" => Self::Security,
            "data_analysis" | "dataanalysis" => Self::DataAnalysis,
            "documentation" => Self::Documentation,
            "incident_response" | "incidentresponse" => Self::IncidentResponse,
            other => Self::Custom(other.to_string()),
        }
    }
}

/// Skills configuration for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkills {
    /// Base skills always loaded (core competencies).
    pub base_skills: Vec<SkillRef>,

    /// Skills available for dynamic activation.
    pub available_skills: Vec<SkillRef>,

    /// Skill discovery mode.
    pub discovery: SkillDiscovery,

    /// Maximum skills that can be active simultaneously.
    pub max_active_skills: usize,
}

impl Default for AgentSkills {
    fn default() -> Self {
        Self {
            base_skills: Vec::new(),
            available_skills: Vec::new(),
            discovery: SkillDiscovery::Explicit,
            max_active_skills: 5,
        }
    }
}

impl AgentSkills {
    /// Create a new skills configuration with explicit discovery.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a base skill.
    pub fn with_base_skill(mut self, skill: SkillRef) -> Self {
        self.base_skills.push(skill);
        self
    }

    /// Add an available skill.
    pub fn with_available_skill(mut self, skill: SkillRef) -> Self {
        self.available_skills.push(skill);
        self
    }

    /// Set the discovery mode.
    pub fn with_discovery(mut self, discovery: SkillDiscovery) -> Self {
        self.discovery = discovery;
        self
    }

    /// Set the maximum active skills.
    pub fn with_max_active(mut self, max: usize) -> Self {
        self.max_active_skills = max;
        self
    }

    /// Get all skill references (base + available).
    pub fn all_skills(&self) -> impl Iterator<Item = &SkillRef> {
        self.base_skills.iter().chain(self.available_skills.iter())
    }
}

/// Reference to a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRef {
    /// Skill identifier (matches SKILL.md name field).
    pub skill_id: String,

    /// Source location.
    pub source: SkillSource,

    /// Override allowed-tools from skill.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_tools_override: Option<Vec<String>>,
}

impl SkillRef {
    /// Create a new skill reference.
    pub fn new(skill_id: impl Into<String>, source: SkillSource) -> Self {
        Self {
            skill_id: skill_id.into(),
            source,
            allowed_tools_override: None,
        }
    }

    /// Create a reference to a builtin skill.
    pub fn builtin(name: impl Into<String>) -> Self {
        let name = name.into();
        Self::new(name.clone(), SkillSource::Builtin { name })
    }

    /// Create a reference to a local skill.
    pub fn local(skill_id: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self::new(skill_id, SkillSource::Local { path: path.into() })
    }

    /// Create a reference to a git-hosted skill.
    pub fn git(skill_id: impl Into<String>, repo: impl Into<String>) -> Self {
        Self::new(
            skill_id,
            SkillSource::Git {
                repo: repo.into(),
                path: None,
                git_ref: None,
            },
        )
    }

    /// Override the allowed tools.
    pub fn with_allowed_tools(mut self, tools: Vec<String>) -> Self {
        self.allowed_tools_override = Some(tools);
        self
    }
}

/// Where to load a skill from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SkillSource {
    /// Built-in skill (bundled with Shiioo).
    Builtin { name: String },

    /// Skill from a local directory.
    Local { path: PathBuf },

    /// Skill from a Git repository.
    Git {
        repo: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        git_ref: Option<String>,
    },

    /// Skill from a registry.
    Registry {
        registry: String,
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        version: Option<String>,
    },
}

/// How skills are discovered and activated.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SkillDiscovery {
    /// Only use explicitly assigned skills.
    Explicit,

    /// Match skills by task description.
    TaskMatching,

    /// Agent can request skills from a library.
    OnDemand { library: Vec<SkillRef> },
}

impl Default for SkillDiscovery {
    fn default() -> Self {
        Self::Explicit
    }
}

/// A parsed skill definition (from SKILL.md).
///
/// This represents the full content of a skill after parsing
/// the SKILL.md file and discovering associated resources.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    /// Skill identifier (from frontmatter `name` field).
    pub name: String,

    /// When to use this skill (from frontmatter `description` field).
    pub description: String,

    /// Skill category for organization and browsing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<SkillCategory>,

    /// License information.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,

    /// Environment requirements.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,

    /// Pre-approved tools (from frontmatter `allowed-tools` field).
    #[serde(default)]
    pub allowed_tools: Vec<String>,

    /// Custom metadata.
    #[serde(default)]
    pub metadata: HashMap<String, String>,

    /// Full instructions (markdown body after frontmatter).
    pub instructions: String,

    /// Available scripts in the skill.
    #[serde(default)]
    pub scripts: Vec<SkillScript>,

    /// Reference documents in the skill.
    #[serde(default)]
    pub references: Vec<SkillReference>,

    /// Asset files in the skill.
    #[serde(default)]
    pub assets: Vec<SkillAsset>,
}

impl Skill {
    /// Create a new skill with minimal required fields.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            category: None,
            license: None,
            compatibility: None,
            allowed_tools: Vec::new(),
            metadata: HashMap::new(),
            instructions: String::new(),
            scripts: Vec::new(),
            references: Vec::new(),
            assets: Vec::new(),
        }
    }

    /// Get skill metadata (name, description, category).
    pub fn metadata(&self) -> SkillMetadata {
        SkillMetadata {
            name: self.name.clone(),
            description: self.description.clone(),
            category: self.category.clone(),
            allowed_tools: self.allowed_tools.clone(),
        }
    }
}

/// Skill metadata for discovery (lightweight, always loaded).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetadata {
    /// Skill name.
    pub name: String,

    /// Skill description.
    pub description: String,

    /// Skill category.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<SkillCategory>,

    /// Pre-approved tools.
    pub allowed_tools: Vec<String>,
}

impl SkillMetadata {
    /// Estimate token count for this metadata.
    pub fn estimated_tokens(&self) -> usize {
        // Rough estimate: ~4 chars per token
        (self.name.len() + self.description.len() + self.allowed_tools.join(" ").len()) / 4
    }
}

/// A script bundled with a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillScript {
    /// Path relative to skill root.
    pub path: String,

    /// Script language (e.g., "python", "bash", "javascript").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,

    /// Description of what the script does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl SkillScript {
    /// Create a new script reference.
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            language: None,
            description: None,
        }
    }

    /// Set the script language.
    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    /// Set the script description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// A reference document bundled with a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillReference {
    /// Path relative to skill root.
    pub path: String,

    /// Title of the reference document.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl SkillReference {
    /// Create a new reference.
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            title: None,
        }
    }

    /// Set the reference title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
}

/// An asset file bundled with a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillAsset {
    /// Path relative to skill root.
    pub path: String,

    /// MIME content type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

impl SkillAsset {
    /// Create a new asset reference.
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            content_type: None,
        }
    }

    /// Set the content type.
    pub fn with_content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }
}

/// Skill loading state for progressive disclosure.
#[derive(Debug, Clone)]
pub enum SkillLoadState {
    /// Only metadata loaded (name, description).
    Metadata(SkillMetadata),

    /// Full skill loaded (instructions + resources).
    Full(Skill),
}

impl SkillLoadState {
    /// Check if this is fully loaded.
    pub fn is_full(&self) -> bool {
        matches!(self, Self::Full(_))
    }

    /// Get the metadata (available in both states).
    /// Returns an owned value to avoid lifetime issues.
    pub fn metadata(&self) -> SkillMetadata {
        match self {
            Self::Metadata(m) => m.clone(),
            Self::Full(s) => SkillMetadata {
                name: s.name.clone(),
                description: s.description.clone(),
                category: s.category.clone(),
                allowed_tools: s.allowed_tools.clone(),
            },
        }
    }

    /// Get the full skill if loaded.
    pub fn full(&self) -> Option<&Skill> {
        match self {
            Self::Full(s) => Some(s),
            Self::Metadata(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_ref_builtin() {
        let skill = SkillRef::builtin("code-review");
        assert_eq!(skill.skill_id, "code-review");
        assert!(matches!(skill.source, SkillSource::Builtin { name } if name == "code-review"));
    }

    #[test]
    fn test_skill_ref_local() {
        let skill = SkillRef::local("my-skill", "/path/to/skill");
        assert_eq!(skill.skill_id, "my-skill");
        assert!(matches!(skill.source, SkillSource::Local { path } if path == PathBuf::from("/path/to/skill")));
    }

    #[test]
    fn test_skill_ref_git() {
        let skill = SkillRef::git("rust-dev", "github.com/company/skills");
        assert_eq!(skill.skill_id, "rust-dev");
        assert!(
            matches!(skill.source, SkillSource::Git { repo, .. } if repo == "github.com/company/skills")
        );
    }

    #[test]
    fn test_agent_skills_builder() {
        let skills = AgentSkills::new()
            .with_base_skill(SkillRef::builtin("code-review"))
            .with_available_skill(SkillRef::builtin("debugging"))
            .with_discovery(SkillDiscovery::TaskMatching)
            .with_max_active(3);

        assert_eq!(skills.base_skills.len(), 1);
        assert_eq!(skills.available_skills.len(), 1);
        assert_eq!(skills.max_active_skills, 3);
        assert!(matches!(skills.discovery, SkillDiscovery::TaskMatching));
    }

    #[test]
    fn test_skill_creation() {
        let skill = Skill::new("pdf-processing", "Extract text and tables from PDF files");
        assert_eq!(skill.name, "pdf-processing");
        assert!(skill.instructions.is_empty());

        let metadata = skill.metadata();
        assert_eq!(metadata.name, "pdf-processing");
    }

    #[test]
    fn test_skill_category_display() {
        assert_eq!(SkillCategory::Engineering.to_string(), "engineering");
        assert_eq!(SkillCategory::DataAnalysis.to_string(), "data_analysis");
        assert_eq!(
            SkillCategory::Custom("my-cat".to_string()).to_string(),
            "my-cat"
        );
    }

    #[test]
    fn test_skill_category_from_str() {
        assert_eq!(
            SkillCategory::from_str_loose("engineering"),
            SkillCategory::Engineering
        );
        assert_eq!(
            SkillCategory::from_str_loose("data_analysis"),
            SkillCategory::DataAnalysis
        );
        assert_eq!(
            SkillCategory::from_str_loose("data-analysis"),
            SkillCategory::DataAnalysis
        );
        assert_eq!(
            SkillCategory::from_str_loose("unknown"),
            SkillCategory::Custom("unknown".to_string())
        );
    }

    #[test]
    fn test_skill_metadata_tokens() {
        let metadata = SkillMetadata {
            name: "test-skill".to_string(),
            description: "A test skill for testing purposes".to_string(),
            category: Some(SkillCategory::Testing),
            allowed_tools: vec!["tool1".to_string(), "tool2".to_string()],
        };

        // Should return some reasonable token estimate
        let tokens = metadata.estimated_tokens();
        assert!(tokens > 0);
        assert!(tokens < 100); // Should be small for this example
    }
}
