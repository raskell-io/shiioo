//! Skill registry for browsing and querying available skills.
//!
//! Provides a categorized view of built-in and imported skills.

use std::collections::HashMap;

use super::{builtin_skills, Skill, SkillCategory, SkillMetadata};

/// Source of a registry entry.
#[derive(Debug, Clone)]
pub enum RegistryEntrySource {
    /// Built-in skill shipped with Shiioo.
    Builtin,
    /// Imported from an external source.
    Imported { origin: String },
}

/// An entry in the skill registry.
#[derive(Debug, Clone)]
pub struct RegistryEntry {
    /// Skill name.
    pub name: String,
    /// Skill description.
    pub description: String,
    /// Skill category.
    pub category: SkillCategory,
    /// Pre-approved tools.
    pub allowed_tools: Vec<String>,
    /// Searchable tags.
    pub tags: Vec<String>,
    /// Where this entry came from.
    pub source: RegistryEntrySource,
}

/// Registry of available skills with category-based querying.
pub struct SkillRegistry {
    entries: Vec<RegistryEntry>,
    skills: HashMap<String, Skill>,
}

impl SkillRegistry {
    /// Create a new registry pre-populated with built-in skills.
    pub fn new() -> Self {
        let mut entries = Vec::new();
        let mut skills = HashMap::new();

        for metadata in builtin_skills::all_metadata() {
            let category = metadata
                .category
                .clone()
                .unwrap_or(SkillCategory::Engineering);

            entries.push(RegistryEntry {
                name: metadata.name.clone(),
                description: metadata.description.clone(),
                category,
                allowed_tools: metadata.allowed_tools.clone(),
                tags: Vec::new(),
                source: RegistryEntrySource::Builtin,
            });

            // Eagerly load built-in skills since they're in-memory anyway
            if let Some(skill) = builtin_skills::get_skill(&metadata.name) {
                skills.insert(metadata.name, skill);
            }
        }

        Self { entries, skills }
    }

    /// List all registry entries.
    pub fn list_entries(&self) -> &[RegistryEntry] {
        &self.entries
    }

    /// Filter entries by category.
    pub fn by_category(&self, category: &SkillCategory) -> Vec<&RegistryEntry> {
        self.entries
            .iter()
            .filter(|e| &e.category == category)
            .collect()
    }

    /// Search entries by query string (matches name, description, and tags).
    pub fn search(&self, query: &str) -> Vec<&RegistryEntry> {
        let query_lower = query.to_lowercase();
        let query_terms: Vec<&str> = query_lower.split_whitespace().collect();

        self.entries
            .iter()
            .filter(|entry| {
                let haystack = format!(
                    "{} {} {}",
                    entry.name,
                    entry.description,
                    entry.tags.join(" ")
                )
                .to_lowercase();

                query_terms.iter().all(|term| haystack.contains(term))
            })
            .collect()
    }

    /// Get a skill by name.
    pub fn get_skill(&self, name: &str) -> Option<&Skill> {
        self.skills.get(name)
    }

    /// Register a new entry and its associated skill.
    pub fn register(&mut self, entry: RegistryEntry, skill: Skill) {
        self.skills.insert(entry.name.clone(), skill);
        self.entries.push(entry);
    }

    /// Get all distinct categories with their entry counts.
    pub fn categories(&self) -> Vec<(SkillCategory, usize)> {
        let mut counts: HashMap<SkillCategory, usize> = HashMap::new();
        for entry in &self.entries {
            *counts.entry(entry.category.clone()).or_insert(0) += 1;
        }
        let mut result: Vec<_> = counts.into_iter().collect();
        result.sort_by(|a, b| a.0.to_string().cmp(&b.0.to_string()));
        result
    }

    /// Get metadata for all entries.
    pub fn all_metadata(&self) -> Vec<SkillMetadata> {
        self.entries
            .iter()
            .map(|e| SkillMetadata {
                name: e.name.clone(),
                description: e.description.clone(),
                category: Some(e.category.clone()),
                allowed_tools: e.allowed_tools.clone(),
            })
            .collect()
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_new_has_builtins() {
        let registry = SkillRegistry::new();
        let entries = registry.list_entries();
        assert!(entries.len() >= 9, "Should have at least 9 built-in skills");
    }

    #[test]
    fn test_registry_by_category() {
        let registry = SkillRegistry::new();
        let engineering = registry.by_category(&SkillCategory::Engineering);
        assert!(
            engineering.len() >= 3,
            "Should have at least 3 engineering skills"
        );

        for entry in &engineering {
            assert_eq!(entry.category, SkillCategory::Engineering);
        }
    }

    #[test]
    fn test_registry_search() {
        let registry = SkillRegistry::new();

        let results = registry.search("code review");
        assert!(!results.is_empty(), "Should find code-review skill");
        assert!(results.iter().any(|e| e.name == "code-review"));

        let results = registry.search("SQL");
        assert!(!results.is_empty(), "Should find sql-queries skill");
        assert!(results.iter().any(|e| e.name == "sql-queries"));
    }

    #[test]
    fn test_registry_get_skill() {
        let registry = SkillRegistry::new();
        let skill = registry.get_skill("code-review");
        assert!(skill.is_some());
        assert!(!skill.unwrap().instructions.is_empty());
    }

    #[test]
    fn test_registry_categories() {
        let registry = SkillRegistry::new();
        let categories = registry.categories();
        assert!(categories.len() >= 5, "Should have multiple categories");
    }

    #[test]
    fn test_registry_register() {
        let mut registry = SkillRegistry::new();
        let initial_count = registry.list_entries().len();

        let entry = RegistryEntry {
            name: "custom-skill".to_string(),
            description: "A custom skill".to_string(),
            category: SkillCategory::Custom("custom".to_string()),
            allowed_tools: vec![],
            tags: vec!["custom".to_string()],
            source: RegistryEntrySource::Imported {
                origin: "test".to_string(),
            },
        };

        let skill = Skill::new("custom-skill", "A custom skill");
        registry.register(entry, skill);

        assert_eq!(registry.list_entries().len(), initial_count + 1);
        assert!(registry.get_skill("custom-skill").is_some());
    }

    #[test]
    fn test_registry_search_by_tag() {
        let mut registry = SkillRegistry::new();

        let entry = RegistryEntry {
            name: "tagged-skill".to_string(),
            description: "A skill".to_string(),
            category: SkillCategory::Engineering,
            allowed_tools: vec![],
            tags: vec!["rust".to_string(), "backend".to_string()],
            source: RegistryEntrySource::Builtin,
        };

        let skill = Skill::new("tagged-skill", "A skill");
        registry.register(entry, skill);

        let results = registry.search("rust backend");
        assert!(!results.is_empty());
        assert!(results.iter().any(|e| e.name == "tagged-skill"));
    }
}
