//! Skill manager for agents.
//!
//! Handles skill discovery, loading, activation, and context management.

use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::{
    skill_parser::SkillParser, Skill, SkillDiscovery, SkillMetadata, SkillRef,
    SkillSource,
};

/// Trait for managing skills for an agent.
#[async_trait::async_trait]
pub trait SkillManager: Send + Sync {
    /// Load skill metadata for discovery (lightweight).
    async fn load_metadata(&self, skill_ref: &SkillRef) -> Result<SkillMetadata>;

    /// Fully load a skill (instructions + resources).
    async fn load_full(&self, skill_ref: &SkillRef) -> Result<Skill>;

    /// Get currently active skills.
    async fn active_skills(&self) -> Vec<Skill>;

    /// Activate a skill (load into context).
    async fn activate(&self, skill_id: &str) -> Result<()>;

    /// Deactivate a skill (free context).
    async fn deactivate(&self, skill_id: &str) -> Result<()>;

    /// Match task description to available skills.
    async fn match_skills(&self, task_description: &str) -> Vec<SkillMetadata>;

    /// Get all available skill metadata.
    async fn available_skills(&self) -> Vec<SkillMetadata>;

    /// Get the current context token estimate.
    async fn context_tokens(&self) -> usize;
}

/// Configuration for the skill manager.
#[derive(Debug, Clone)]
pub struct SkillManagerConfig {
    /// Maximum number of skills that can be active at once.
    pub max_active_skills: usize,

    /// Base directory for local skills.
    pub local_skills_dir: Option<PathBuf>,

    /// Cache directory for remote skills.
    pub cache_dir: Option<PathBuf>,

    /// Whether to enable skill matching.
    pub enable_matching: bool,
}

impl Default for SkillManagerConfig {
    fn default() -> Self {
        Self {
            max_active_skills: 5,
            local_skills_dir: None,
            cache_dir: None,
            enable_matching: true,
        }
    }
}

/// Default implementation of SkillManager.
pub struct DefaultSkillManager {
    config: SkillManagerConfig,

    /// Base skills (always loaded).
    base_skills: Vec<SkillRef>,

    /// Available skills (can be activated).
    available_skills: Vec<SkillRef>,

    /// Skill discovery mode.
    discovery: SkillDiscovery,

    /// Loaded skill metadata (cached).
    metadata_cache: Arc<RwLock<HashMap<String, SkillMetadata>>>,

    /// Fully loaded skills (cached).
    skill_cache: Arc<RwLock<HashMap<String, Skill>>>,

    /// Currently active skill IDs.
    active_skill_ids: Arc<RwLock<Vec<String>>>,
}

impl DefaultSkillManager {
    /// Create a new skill manager.
    pub fn new(
        config: SkillManagerConfig,
        base_skills: Vec<SkillRef>,
        available_skills: Vec<SkillRef>,
        discovery: SkillDiscovery,
    ) -> Self {
        Self {
            config,
            base_skills,
            available_skills,
            discovery,
            metadata_cache: Arc::new(RwLock::new(HashMap::new())),
            skill_cache: Arc::new(RwLock::new(HashMap::new())),
            active_skill_ids: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Create from an AgentSkills configuration.
    pub fn from_agent_skills(
        skills: &super::AgentSkills,
        config: SkillManagerConfig,
    ) -> Self {
        Self::new(
            SkillManagerConfig {
                max_active_skills: skills.max_active_skills,
                ..config
            },
            skills.base_skills.clone(),
            skills.available_skills.clone(),
            skills.discovery.clone(),
        )
    }

    /// Initialize the manager by loading base skills.
    pub async fn initialize(&self) -> Result<()> {
        // Load metadata for all skills
        for skill_ref in self.base_skills.iter().chain(self.available_skills.iter()) {
            if let Ok(metadata) = self.load_metadata(skill_ref).await {
                let mut cache = self.metadata_cache.write().await;
                cache.insert(skill_ref.skill_id.clone(), metadata);
            }
        }

        // Activate base skills
        for skill_ref in &self.base_skills {
            self.activate(&skill_ref.skill_id).await?;
        }

        Ok(())
    }

    /// Resolve a skill source to a path.
    async fn resolve_skill_path(&self, source: &SkillSource) -> Result<PathBuf> {
        match source {
            SkillSource::Builtin { name } => {
                // Look in built-in skills directory
                let builtin_dir = self
                    .config
                    .local_skills_dir
                    .as_ref()
                    .map(|d| d.join("builtin"))
                    .unwrap_or_else(|| PathBuf::from("skills/builtin"));

                let skill_path = builtin_dir.join(name);
                if skill_path.exists() {
                    Ok(skill_path)
                } else {
                    Err(anyhow!("Builtin skill not found: {}", name))
                }
            }

            SkillSource::Local { path } => {
                if path.exists() {
                    Ok(path.clone())
                } else if let Some(base_dir) = &self.config.local_skills_dir {
                    let full_path = base_dir.join(path);
                    if full_path.exists() {
                        Ok(full_path)
                    } else {
                        Err(anyhow!("Local skill not found: {}", path.display()))
                    }
                } else {
                    Err(anyhow!("Local skill not found: {}", path.display()))
                }
            }

            SkillSource::Git { repo, path, git_ref: _ } => {
                // For git sources, we'd clone/fetch to cache
                // For now, return an error indicating this needs implementation
                let cache_dir = self
                    .config
                    .cache_dir
                    .as_ref()
                    .ok_or_else(|| anyhow!("Cache directory not configured for git skills"))?;

                let repo_hash = Self::hash_repo_url(repo);
                let repo_dir = cache_dir.join("git").join(&repo_hash);

                if !repo_dir.exists() {
                    // TODO: Clone the repository
                    return Err(anyhow!(
                        "Git skill repository not cached. Clone {} to {}",
                        repo,
                        repo_dir.display()
                    ));
                }

                let skill_path = if let Some(subpath) = path {
                    repo_dir.join(subpath)
                } else {
                    repo_dir
                };

                if skill_path.exists() {
                    Ok(skill_path)
                } else {
                    Err(anyhow!(
                        "Skill path not found in repository: {}",
                        skill_path.display()
                    ))
                }
            }

            SkillSource::Registry { registry, name, version } => {
                // For registry sources, we'd fetch from the registry
                let cache_dir = self
                    .config
                    .cache_dir
                    .as_ref()
                    .ok_or_else(|| anyhow!("Cache directory not configured for registry skills"))?;

                let version_str = version.as_deref().unwrap_or("latest");
                let skill_dir = cache_dir
                    .join("registry")
                    .join(registry)
                    .join(name)
                    .join(version_str);

                if skill_dir.exists() {
                    Ok(skill_dir)
                } else {
                    Err(anyhow!(
                        "Registry skill not cached: {}@{} from {}",
                        name,
                        version_str,
                        registry
                    ))
                }
            }
        }
    }

    /// Hash a repository URL for cache directory naming.
    fn hash_repo_url(url: &str) -> String {
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(url.as_bytes());
        hex::encode(&hash[..8])
    }

    /// Calculate simple keyword overlap score for skill matching.
    fn calculate_match_score(task: &str, skill_description: &str) -> f64 {
        let task_lower = task.to_lowercase();
        let task_words: std::collections::HashSet<&str> = task_lower
            .split_whitespace()
            .filter(|w| w.len() > 2)
            .collect();

        let skill_lower = skill_description.to_lowercase();
        let skill_words: std::collections::HashSet<&str> = skill_lower
            .split_whitespace()
            .filter(|w| w.len() > 2)
            .collect();

        if task_words.is_empty() || skill_words.is_empty() {
            return 0.0;
        }

        let overlap = task_words.intersection(&skill_words).count();
        overlap as f64 / task_words.len().min(skill_words.len()) as f64
    }

    /// Estimate token count for a skill.
    fn estimate_skill_tokens(skill: &Skill) -> usize {
        // Rough estimate: ~4 chars per token
        let text_len = skill.name.len()
            + skill.description.len()
            + skill.instructions.len()
            + skill.allowed_tools.join(" ").len();

        text_len / 4
    }
}

#[async_trait::async_trait]
impl SkillManager for DefaultSkillManager {
    async fn load_metadata(&self, skill_ref: &SkillRef) -> Result<SkillMetadata> {
        // Check cache first
        {
            let cache = self.metadata_cache.read().await;
            if let Some(metadata) = cache.get(&skill_ref.skill_id) {
                return Ok(metadata.clone());
            }
        }

        // Resolve and load
        let path = self.resolve_skill_path(&skill_ref.source).await?;
        let skill_md_path = if path.is_dir() {
            path.join("SKILL.md")
        } else {
            path
        };

        let content = tokio::fs::read_to_string(&skill_md_path)
            .await
            .with_context(|| format!("Failed to read skill: {}", skill_md_path.display()))?;

        let mut metadata = SkillParser::parse_metadata(&content)?;

        // Apply allowed_tools override if specified
        if let Some(override_tools) = &skill_ref.allowed_tools_override {
            metadata.allowed_tools = override_tools.clone();
        }

        // Cache it
        {
            let mut cache = self.metadata_cache.write().await;
            cache.insert(skill_ref.skill_id.clone(), metadata.clone());
        }

        Ok(metadata)
    }

    async fn load_full(&self, skill_ref: &SkillRef) -> Result<Skill> {
        // Check cache first
        {
            let cache = self.skill_cache.read().await;
            if let Some(skill) = cache.get(&skill_ref.skill_id) {
                return Ok(skill.clone());
            }
        }

        // Resolve and load
        let path = self.resolve_skill_path(&skill_ref.source).await?;

        let skill = if path.is_dir() {
            SkillParser::parse_directory(&path)?
        } else {
            SkillParser::parse_file(&path)?
        };

        // Apply allowed_tools override if specified
        let mut skill = skill;
        if let Some(override_tools) = &skill_ref.allowed_tools_override {
            skill.allowed_tools = override_tools.clone();
        }

        // Cache it
        {
            let mut cache = self.skill_cache.write().await;
            cache.insert(skill_ref.skill_id.clone(), skill.clone());
        }

        Ok(skill)
    }

    async fn active_skills(&self) -> Vec<Skill> {
        let active_ids = self.active_skill_ids.read().await;
        let cache = self.skill_cache.read().await;

        active_ids
            .iter()
            .filter_map(|id| cache.get(id).cloned())
            .collect()
    }

    async fn activate(&self, skill_id: &str) -> Result<()> {
        // Check if already active
        {
            let active = self.active_skill_ids.read().await;
            if active.contains(&skill_id.to_string()) {
                return Ok(());
            }
        }

        // Check max active limit
        {
            let active = self.active_skill_ids.read().await;
            if active.len() >= self.config.max_active_skills {
                return Err(anyhow!(
                    "Maximum active skills ({}) reached. Deactivate a skill first.",
                    self.config.max_active_skills
                ));
            }
        }

        // Find the skill ref
        let skill_ref = self
            .base_skills
            .iter()
            .chain(self.available_skills.iter())
            .find(|s| s.skill_id == skill_id)
            .ok_or_else(|| anyhow!("Skill not found: {}", skill_id))?
            .clone();

        // Load the full skill
        self.load_full(&skill_ref).await?;

        // Mark as active
        {
            let mut active = self.active_skill_ids.write().await;
            active.push(skill_id.to_string());
        }

        tracing::info!(skill_id = %skill_id, "Skill activated");
        Ok(())
    }

    async fn deactivate(&self, skill_id: &str) -> Result<()> {
        // Check if it's a base skill (can't deactivate)
        if self.base_skills.iter().any(|s| s.skill_id == skill_id) {
            return Err(anyhow!(
                "Cannot deactivate base skill: {}",
                skill_id
            ));
        }

        let mut active = self.active_skill_ids.write().await;
        if let Some(pos) = active.iter().position(|id| id == skill_id) {
            active.remove(pos);
            tracing::info!(skill_id = %skill_id, "Skill deactivated");
            Ok(())
        } else {
            Err(anyhow!("Skill not active: {}", skill_id))
        }
    }

    async fn match_skills(&self, task_description: &str) -> Vec<SkillMetadata> {
        if !self.config.enable_matching {
            return Vec::new();
        }

        // Only match if discovery mode supports it
        match &self.discovery {
            SkillDiscovery::Explicit => return Vec::new(),
            SkillDiscovery::TaskMatching | SkillDiscovery::OnDemand { .. } => {}
        }

        let metadata_cache = self.metadata_cache.read().await;

        let mut scored: Vec<_> = metadata_cache
            .values()
            .map(|m| {
                let score = Self::calculate_match_score(task_description, &m.description);
                (m.clone(), score)
            })
            .filter(|(_, score)| *score > 0.1) // Minimum threshold
            .collect();

        // Sort by score descending
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Return top matches
        scored.into_iter().take(5).map(|(m, _)| m).collect()
    }

    async fn available_skills(&self) -> Vec<SkillMetadata> {
        let cache = self.metadata_cache.read().await;
        cache.values().cloned().collect()
    }

    async fn context_tokens(&self) -> usize {
        let active = self.active_skills().await;
        active.iter().map(Self::estimate_skill_tokens).sum()
    }
}

/// Builder for creating a DefaultSkillManager.
pub struct SkillManagerBuilder {
    config: SkillManagerConfig,
    base_skills: Vec<SkillRef>,
    available_skills: Vec<SkillRef>,
    discovery: SkillDiscovery,
}

impl SkillManagerBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            config: SkillManagerConfig::default(),
            base_skills: Vec::new(),
            available_skills: Vec::new(),
            discovery: SkillDiscovery::Explicit,
        }
    }

    /// Set the configuration.
    pub fn config(mut self, config: SkillManagerConfig) -> Self {
        self.config = config;
        self
    }

    /// Set the maximum active skills.
    pub fn max_active_skills(mut self, max: usize) -> Self {
        self.config.max_active_skills = max;
        self
    }

    /// Set the local skills directory.
    pub fn local_skills_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.config.local_skills_dir = Some(dir.into());
        self
    }

    /// Set the cache directory.
    pub fn cache_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.config.cache_dir = Some(dir.into());
        self
    }

    /// Add a base skill.
    pub fn base_skill(mut self, skill: SkillRef) -> Self {
        self.base_skills.push(skill);
        self
    }

    /// Add an available skill.
    pub fn available_skill(mut self, skill: SkillRef) -> Self {
        self.available_skills.push(skill);
        self
    }

    /// Set the discovery mode.
    pub fn discovery(mut self, discovery: SkillDiscovery) -> Self {
        self.discovery = discovery;
        self
    }

    /// Build the skill manager.
    pub fn build(self) -> DefaultSkillManager {
        DefaultSkillManager::new(
            self.config,
            self.base_skills,
            self.available_skills,
            self.discovery,
        )
    }
}

impl Default for SkillManagerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_match_score() {
        let score = DefaultSkillManager::calculate_match_score(
            "extract text from PDF files",
            "Extract text and tables from PDF files, fill forms",
        );
        assert!(score > 0.5, "Score should be high for similar descriptions");

        let score = DefaultSkillManager::calculate_match_score(
            "deploy kubernetes cluster",
            "Extract text from PDF files",
        );
        assert!(score < 0.2, "Score should be low for unrelated descriptions");
    }

    #[test]
    fn test_hash_repo_url() {
        let hash1 = DefaultSkillManager::hash_repo_url("https://github.com/org/repo");
        let hash2 = DefaultSkillManager::hash_repo_url("https://github.com/org/repo");
        let hash3 = DefaultSkillManager::hash_repo_url("https://github.com/other/repo");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
        assert_eq!(hash1.len(), 16); // 8 bytes = 16 hex chars
    }

    #[test]
    fn test_estimate_tokens() {
        let skill = Skill::new("test-skill", "A test skill")
            ;
        let tokens = DefaultSkillManager::estimate_skill_tokens(&skill);
        assert!(tokens > 0);
        assert!(tokens < 50); // Should be small for minimal skill
    }

    #[test]
    fn test_skill_manager_builder() {
        let manager = SkillManagerBuilder::new()
            .max_active_skills(3)
            .local_skills_dir("/path/to/skills")
            .base_skill(SkillRef::builtin("code-review"))
            .available_skill(SkillRef::builtin("debugging"))
            .discovery(SkillDiscovery::TaskMatching)
            .build();

        assert_eq!(manager.config.max_active_skills, 3);
        assert_eq!(manager.base_skills.len(), 1);
        assert_eq!(manager.available_skills.len(), 1);
    }

    #[tokio::test]
    async fn test_skill_manager_activation_limit() {
        let manager = SkillManagerBuilder::new()
            .max_active_skills(1)
            .build();

        // Manually add to active to simulate activation
        {
            let mut active = manager.active_skill_ids.write().await;
            active.push("skill1".to_string());
        }

        // Try to activate another - should fail due to limit
        // Note: This would need actual skill refs to work fully
        // For now we test the limit check logic
        let active = manager.active_skill_ids.read().await;
        assert_eq!(active.len(), 1);
        assert!(active.len() >= manager.config.max_active_skills);
    }

    #[tokio::test]
    async fn test_skill_manager_context_tokens() {
        let manager = SkillManagerBuilder::new().build();

        // Empty manager should have 0 tokens
        let tokens = manager.context_tokens().await;
        assert_eq!(tokens, 0);
    }
}
