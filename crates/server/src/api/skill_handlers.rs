//! Skill registry, git import, and format conversion API handlers.

use super::ApiResult;
use crate::config::AppState;
use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use shiioo_core::agent::{
    ImportRequest, ImportedSkill, ScanResult, SkillCategory, SkillMetadata,
};
use std::sync::Arc;

// ============================================================================
// Registry Endpoints
// ============================================================================

/// List registry skills with optional category/search filters.
pub async fn list_registry_skills(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListRegistryParams>,
) -> ApiResult<Json<ListRegistryResponse>> {
    let registry = state.skill_registry.read().await;

    let entries: Vec<SkillMetadata> = if let Some(ref category) = params.category {
        let cat = SkillCategory::from_str_loose(category);
        registry
            .by_category(&cat)
            .into_iter()
            .map(|e| SkillMetadata {
                name: e.name.clone(),
                description: e.description.clone(),
                category: Some(e.category.clone()),
                allowed_tools: e.allowed_tools.clone(),
            })
            .collect()
    } else if let Some(ref query) = params.search {
        registry
            .search(query)
            .into_iter()
            .map(|e| SkillMetadata {
                name: e.name.clone(),
                description: e.description.clone(),
                category: Some(e.category.clone()),
                allowed_tools: e.allowed_tools.clone(),
            })
            .collect()
    } else {
        registry.all_metadata()
    };

    Ok(Json(ListRegistryResponse { skills: entries }))
}

#[derive(Debug, Deserialize)]
pub struct ListRegistryParams {
    pub category: Option<String>,
    pub search: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ListRegistryResponse {
    pub skills: Vec<SkillMetadata>,
}

/// List all categories with skill counts.
pub async fn list_categories(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<ListCategoriesResponse>> {
    let registry = state.skill_registry.read().await;
    let categories: Vec<CategoryInfo> = registry
        .categories()
        .into_iter()
        .map(|(cat, count)| CategoryInfo {
            name: cat.to_string(),
            count,
        })
        .collect();

    Ok(Json(ListCategoriesResponse { categories }))
}

#[derive(Debug, Serialize)]
pub struct ListCategoriesResponse {
    pub categories: Vec<CategoryInfo>,
}

#[derive(Debug, Serialize)]
pub struct CategoryInfo {
    pub name: String,
    pub count: usize,
}

// ============================================================================
// Git Import Endpoints
// ============================================================================

/// Scan a git repository for importable skill files.
pub async fn scan_git_repo(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ScanGitRepoRequest>,
) -> ApiResult<Json<ScanResult>> {
    let cache_dir = state.skills_cache_dir.clone();
    let local_dir = state.local_skills_dir.clone();

    let importer =
        shiioo_core::agent::GitImporter::new(cache_dir, local_dir);

    let repo_path = importer
        .clone_repo(&req.repo_url, req.git_ref.as_deref())
        .await?;
    let files = importer.scan_repo(&repo_path)?;

    Ok(Json(ScanResult {
        repo_url: req.repo_url,
        git_ref: req.git_ref.unwrap_or_else(|| "HEAD".to_string()),
        files,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ScanGitRepoRequest {
    pub repo_url: String,
    pub git_ref: Option<String>,
}

/// Import selected files from a scanned git repository.
pub async fn import_from_git(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ImportRequest>,
) -> ApiResult<Json<ImportFromGitResponse>> {
    let cache_dir = state.skills_cache_dir.clone();
    let local_dir = state.local_skills_dir.clone();

    let importer =
        shiioo_core::agent::GitImporter::new(cache_dir, local_dir.clone());

    let result = importer.import_files(&req).await?;

    // Register imported skills in the registry
    let mut registry = state.skill_registry.write().await;
    for imported in &result.imported {
        let skill_dir = local_dir.join(&imported.skill_name);
        let skill_md = skill_dir.join("SKILL.md");
        if let Ok(content) = std::fs::read_to_string(&skill_md) {
            if let Ok(skill) = shiioo_core::agent::SkillParser::parse(&content) {
                let category = skill
                    .category
                    .clone()
                    .unwrap_or(SkillCategory::Custom("imported".to_string()));

                registry.register(
                    shiioo_core::agent::RegistryEntry {
                        name: skill.name.clone(),
                        description: skill.description.clone(),
                        category,
                        allowed_tools: skill.allowed_tools.clone(),
                        tags: vec!["imported".to_string()],
                        source: shiioo_core::agent::RegistryEntrySource::Imported {
                            origin: req.repo_url.clone(),
                        },
                    },
                    skill,
                );
            }
        }
    }

    Ok(Json(ImportFromGitResponse {
        imported: result.imported,
        errors: result
            .errors
            .into_iter()
            .map(|e| ImportErrorDto {
                path: e.path,
                error: e.error,
            })
            .collect(),
    }))
}

#[derive(Debug, Serialize)]
pub struct ImportFromGitResponse {
    pub imported: Vec<ImportedSkill>,
    pub errors: Vec<ImportErrorDto>,
}

#[derive(Debug, Serialize)]
pub struct ImportErrorDto {
    pub path: String,
    pub error: String,
}

// ============================================================================
// Format Conversion Endpoint
// ============================================================================

/// Convert raw content to SKILL.md format.
pub async fn convert_format(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ConvertFormatRequest>,
) -> ApiResult<Json<ConvertFormatResponse>> {
    let skill = if let Some(ref format) = req.format {
        state
            .format_converter_registry
            .convert_with(format, &req.content)?
    } else {
        state
            .format_converter_registry
            .auto_detect_and_convert(&req.content)?
    };

    Ok(Json(ConvertFormatResponse {
        name: skill.name.clone(),
        description: skill.description.clone(),
        category: skill.category.as_ref().map(|c| c.to_string()),
        allowed_tools: skill.allowed_tools.clone(),
        instructions_preview: if skill.instructions.len() > 500 {
            format!("{}...", &skill.instructions[..500])
        } else {
            skill.instructions.clone()
        },
    }))
}

#[derive(Debug, Deserialize)]
pub struct ConvertFormatRequest {
    pub content: String,
    pub format: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ConvertFormatResponse {
    pub name: String,
    pub description: String,
    pub category: Option<String>,
    pub allowed_tools: Vec<String>,
    pub instructions_preview: String,
}
