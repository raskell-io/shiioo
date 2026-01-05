use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::types::{StepSpec, WorkflowSpec};

/// Advanced workflow pattern types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AdvancedPattern {
    /// Execute same step for each item in parallel
    ParallelForEach {
        /// Items to iterate over (JSON array)
        items: Vec<serde_json::Value>,
        /// Step template to execute for each item
        step_template: Box<StepSpec>,
        /// Maximum parallelism (0 = unlimited)
        max_parallelism: usize,
    },
    /// Conditional branch execution
    ConditionalBranch {
        /// Condition to evaluate (simple expression)
        condition: String,
        /// Steps to execute if condition is true
        if_steps: Vec<StepSpec>,
        /// Steps to execute if condition is false
        else_steps: Vec<StepSpec>,
    },
    /// Dynamic DAG generation at runtime
    DynamicDAG {
        /// Generator function that produces workflow steps
        generator_step: Box<StepSpec>,
        /// Whether to execute generated steps immediately
        execute_generated: bool,
    },
    /// Loop until condition is met
    Loop {
        /// Loop condition (simple expression)
        condition: String,
        /// Maximum iterations (safety limit)
        max_iterations: usize,
        /// Steps to execute in each iteration
        loop_steps: Vec<StepSpec>,
    },
}

/// Parallel-for-each execution builder
pub struct ParallelForEachBuilder {
    items: Vec<serde_json::Value>,
    step_template: Option<StepSpec>,
    max_parallelism: usize,
}

impl ParallelForEachBuilder {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            step_template: None,
            max_parallelism: 0, // unlimited
        }
    }

    pub fn items(mut self, items: Vec<serde_json::Value>) -> Self {
        self.items = items;
        self
    }

    pub fn step_template(mut self, step: StepSpec) -> Self {
        self.step_template = Some(step);
        self
    }

    pub fn max_parallelism(mut self, max: usize) -> Self {
        self.max_parallelism = max;
        self
    }

    pub fn build(self) -> anyhow::Result<AdvancedPattern> {
        let step_template = self
            .step_template
            .ok_or_else(|| anyhow::anyhow!("Step template is required"))?;

        Ok(AdvancedPattern::ParallelForEach {
            items: self.items,
            step_template: Box::new(step_template),
            max_parallelism: self.max_parallelism,
        })
    }
}

impl Default for ParallelForEachBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Expand parallel-for-each pattern into concrete steps
pub fn expand_parallel_foreach(
    items: &[serde_json::Value],
    step_template: &StepSpec,
    _max_parallelism: usize,
) -> Vec<StepSpec> {
    items
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let mut step = step_template.clone();

            // Replace {{item}} placeholder in action
            if let crate::types::StepAction::AgentTask { prompt } = &step.action {
                let item_str = serde_json::to_string(item).unwrap_or_default();
                let new_prompt = prompt.replace("{{item}}", &item_str);
                step.action = crate::types::StepAction::AgentTask { prompt: new_prompt };
            }

            // Update step ID to include index
            step.id = crate::types::StepId(format!("{}_{}", step.id.0, idx));

            // Update name to include index
            step.name = format!("{} (item {})", step.name, idx);

            step
        })
        .collect()
}

/// Simple condition evaluator
/// Supports basic comparisons: ==, !=, >, <, >=, <=
pub fn evaluate_condition(
    condition: &str,
    context: &HashMap<String, String>,
) -> anyhow::Result<bool> {
    let condition = condition.trim();

    // Support simple variable existence check
    if !condition.contains(' ') {
        return Ok(context.contains_key(condition));
    }

    // Parse basic comparison
    let parts: Vec<&str> = if condition.contains("==") {
        condition.split("==").collect()
    } else if condition.contains("!=") {
        condition.split("!=").collect()
    } else if condition.contains(">=") {
        condition.split(">=").collect()
    } else if condition.contains("<=") {
        condition.split("<=").collect()
    } else if condition.contains('>') {
        condition.split('>').collect()
    } else if condition.contains('<') {
        condition.split('<').collect()
    } else {
        return Err(anyhow::anyhow!("Unsupported condition: {}", condition));
    };

    if parts.len() != 2 {
        return Err(anyhow::anyhow!("Invalid condition format: {}", condition));
    }

    let left = parts[0].trim();
    let right = parts[1].trim();

    // Resolve variables
    let left_val = context.get(left).map(|s| s.as_str()).unwrap_or(left);
    let right_val = context.get(right).map(|s| s.as_str()).unwrap_or(right);

    // Determine operator
    let result = if condition.contains("==") {
        left_val == right_val
    } else if condition.contains("!=") {
        left_val != right_val
    } else if condition.contains(">=") {
        let left_num = left_val.parse::<f64>().ok();
        let right_num = right_val.parse::<f64>().ok();
        match (left_num, right_num) {
            (Some(l), Some(r)) => l >= r,
            _ => left_val >= right_val,
        }
    } else if condition.contains("<=") {
        let left_num = left_val.parse::<f64>().ok();
        let right_num = right_val.parse::<f64>().ok();
        match (left_num, right_num) {
            (Some(l), Some(r)) => l <= r,
            _ => left_val <= right_val,
        }
    } else if condition.contains('>') {
        let left_num = left_val.parse::<f64>().ok();
        let right_num = right_val.parse::<f64>().ok();
        match (left_num, right_num) {
            (Some(l), Some(r)) => l > r,
            _ => left_val > right_val,
        }
    } else if condition.contains('<') {
        let left_num = left_val.parse::<f64>().ok();
        let right_num = right_val.parse::<f64>().ok();
        match (left_num, right_num) {
            (Some(l), Some(r)) => l < r,
            _ => left_val < right_val,
        }
    } else {
        false
    };

    Ok(result)
}

/// Workflow versioning information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowVersion {
    pub workflow_id: String,
    pub version: u32,
    pub spec: WorkflowSpec,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub created_by: String,
    pub changelog: String,
    pub is_deprecated: bool,
}

/// Workflow version manager
pub struct WorkflowVersionManager {
    versions: HashMap<String, Vec<WorkflowVersion>>,
}

impl WorkflowVersionManager {
    pub fn new() -> Self {
        Self {
            versions: HashMap::new(),
        }
    }

    /// Register a new workflow version
    pub fn register_version(
        &mut self,
        workflow_id: String,
        spec: WorkflowSpec,
        created_by: String,
        changelog: String,
    ) -> WorkflowVersion {
        let versions = self.versions.entry(workflow_id.clone()).or_default();

        let version_number = versions.len() as u32 + 1;

        let version = WorkflowVersion {
            workflow_id: workflow_id.clone(),
            version: version_number,
            spec,
            created_at: chrono::Utc::now(),
            created_by,
            changelog,
            is_deprecated: false,
        };

        versions.push(version.clone());

        tracing::info!(
            "Registered workflow version: {} v{}",
            workflow_id,
            version_number
        );

        version
    }

    /// Get latest version of a workflow
    pub fn get_latest_version(&self, workflow_id: &str) -> Option<&WorkflowVersion> {
        self.versions
            .get(workflow_id)
            .and_then(|versions| versions.iter().filter(|v| !v.is_deprecated).last())
    }

    /// Get specific version of a workflow
    pub fn get_version(&self, workflow_id: &str, version: u32) -> Option<&WorkflowVersion> {
        self.versions.get(workflow_id).and_then(|versions| {
            versions.iter().find(|v| v.version == version)
        })
    }

    /// List all versions of a workflow
    pub fn list_versions(&self, workflow_id: &str) -> Vec<&WorkflowVersion> {
        self.versions
            .get(workflow_id)
            .map(|versions| versions.iter().collect())
            .unwrap_or_default()
    }

    /// Deprecate a specific version
    pub fn deprecate_version(&mut self, workflow_id: &str, version: u32) -> anyhow::Result<()> {
        let versions = self
            .versions
            .get_mut(workflow_id)
            .ok_or_else(|| anyhow::anyhow!("Workflow not found: {}", workflow_id))?;

        let version_entry = versions
            .iter_mut()
            .find(|v| v.version == version)
            .ok_or_else(|| anyhow::anyhow!("Version {} not found", version))?;

        version_entry.is_deprecated = true;

        tracing::info!("Deprecated workflow version: {} v{}", workflow_id, version);

        Ok(())
    }
}

impl Default for WorkflowVersionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{RoleId, StepAction, StepId, StepSpec};

    #[test]
    fn test_parallel_foreach_expansion() {
        let items = vec![
            serde_json::json!({"name": "item1"}),
            serde_json::json!({"name": "item2"}),
            serde_json::json!({"name": "item3"}),
        ];

        let template = StepSpec {
            id: StepId("process".to_string()),
            name: "Process Item".to_string(),
            description: Some("Process an item".to_string()),
            role: RoleId("worker".to_string()),
            action: StepAction::AgentTask {
                prompt: "Process item: {{item}}".to_string(),
            },
            timeout_secs: Some(300),
            retry_policy: None,
            requires_approval: false,
        };

        let steps = expand_parallel_foreach(&items, &template, 0);

        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].id.0, "process_0");
        assert_eq!(steps[1].id.0, "process_1");
        assert_eq!(steps[2].id.0, "process_2");

        // Check prompts contain the items
        if let StepAction::AgentTask { prompt } = &steps[0].action {
            assert!(prompt.contains("item1"));
        }
        if let StepAction::AgentTask { prompt } = &steps[1].action {
            assert!(prompt.contains("item2"));
        }
        if let StepAction::AgentTask { prompt } = &steps[2].action {
            assert!(prompt.contains("item3"));
        }
    }

    #[test]
    fn test_evaluate_condition_equality() {
        let mut context = HashMap::new();
        context.insert("status".to_string(), "complete".to_string());

        assert!(evaluate_condition("status == complete", &context).unwrap());
        assert!(!evaluate_condition("status == pending", &context).unwrap());
        assert!(evaluate_condition("status != pending", &context).unwrap());
    }

    #[test]
    fn test_evaluate_condition_numeric() {
        let mut context = HashMap::new();
        context.insert("count".to_string(), "5".to_string());

        assert!(evaluate_condition("count > 3", &context).unwrap());
        assert!(evaluate_condition("count >= 5", &context).unwrap());
        assert!(evaluate_condition("count < 10", &context).unwrap());
        assert!(evaluate_condition("count <= 5", &context).unwrap());
    }

    #[test]
    fn test_evaluate_condition_variable_existence() {
        let mut context = HashMap::new();
        context.insert("flag".to_string(), "true".to_string());

        assert!(evaluate_condition("flag", &context).unwrap());
        assert!(!evaluate_condition("missing", &context).unwrap());
    }

    #[test]
    fn test_workflow_version_manager() {
        let mut manager = WorkflowVersionManager::new();

        let spec1 = WorkflowSpec {
            steps: vec![],
            dependencies: HashMap::new(),
        };

        let spec2 = WorkflowSpec {
            steps: vec![],
            dependencies: HashMap::new(),
        };

        // Register first version
        manager.register_version(
            "test-workflow".to_string(),
            spec1.clone(),
            "user1".to_string(),
            "Initial version".to_string(),
        );

        // Register second version
        manager.register_version(
            "test-workflow".to_string(),
            spec2.clone(),
            "user2".to_string(),
            "Updated parallelism".to_string(),
        );

        // Get latest version
        let latest = manager.get_latest_version("test-workflow").unwrap();
        assert_eq!(latest.version, 2);

        // Get specific version
        let v1 = manager.get_version("test-workflow", 1).unwrap();
        assert_eq!(v1.version, 1);

        // List all versions
        let versions = manager.list_versions("test-workflow");
        assert_eq!(versions.len(), 2);
    }

    #[test]
    fn test_deprecate_version() {
        let mut manager = WorkflowVersionManager::new();

        let spec = WorkflowSpec {
            steps: vec![],
            dependencies: HashMap::new(),
        };

        manager.register_version(
            "test-workflow".to_string(),
            spec.clone(),
            "user1".to_string(),
            "Initial version".to_string(),
        );

        manager.register_version(
            "test-workflow".to_string(),
            spec.clone(),
            "user2".to_string(),
            "Second version".to_string(),
        );

        // Deprecate version 1
        manager.deprecate_version("test-workflow", 1).unwrap();

        // Latest should now be version 2 (version 1 is deprecated)
        let latest = manager.get_latest_version("test-workflow").unwrap();
        assert_eq!(latest.version, 2);

        // But we can still get deprecated version directly
        let v1 = manager.get_version("test-workflow", 1).unwrap();
        assert!(v1.is_deprecated);
    }

    #[test]
    fn test_parallel_foreach_builder() {
        let items = vec![
            serde_json::json!(1),
            serde_json::json!(2),
            serde_json::json!(3),
        ];

        let step = StepSpec {
            id: StepId("test".to_string()),
            name: "Test Step".to_string(),
            description: Some("Test step".to_string()),
            role: RoleId("worker".to_string()),
            action: StepAction::AgentTask {
                prompt: "Process {{item}}".to_string(),
            },
            timeout_secs: Some(300),
            retry_policy: None,
            requires_approval: false,
        };

        let pattern = ParallelForEachBuilder::new()
            .items(items.clone())
            .step_template(step)
            .max_parallelism(5)
            .build()
            .unwrap();

        match pattern {
            AdvancedPattern::ParallelForEach {
                items: pattern_items,
                max_parallelism,
                ..
            } => {
                assert_eq!(pattern_items.len(), 3);
                assert_eq!(max_parallelism, 5);
            }
            _ => panic!("Wrong pattern type"),
        }
    }
}
