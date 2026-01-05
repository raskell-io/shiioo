// Process template system for reusable workflows

use crate::types::{
    ProcessTemplate, StepAction, TemplateId, TemplateInstance, TemplateParameter,
    TemplateParameterType, WorkflowSpec,
};
use anyhow::{Context, Result};
use std::collections::HashMap;

/// Template processor for instantiating workflow templates
pub struct TemplateProcessor;

impl TemplateProcessor {
    /// Instantiate a template with given parameters
    pub fn instantiate(
        template: &ProcessTemplate,
        instance: &TemplateInstance,
    ) -> Result<WorkflowSpec> {
        // Validate that all required parameters are provided
        for param in &template.parameters {
            if param.required && !instance.parameters.contains_key(&param.name) {
                if param.default_value.is_none() {
                    anyhow::bail!("Required parameter '{}' not provided", param.name);
                }
            }
        }

        // Build parameter map with defaults
        let mut param_values: HashMap<String, String> = HashMap::new();
        for param in &template.parameters {
            let value = instance
                .parameters
                .get(&param.name)
                .or(param.default_value.as_ref())
                .context(format!("Parameter '{}' not provided and has no default", param.name))?;

            // Validate parameter type
            Self::validate_parameter(param, value)?;

            param_values.insert(param.name.clone(), value.clone());
        }

        // Instantiate the workflow by replacing parameters
        let mut workflow = template.workflow_template.clone();

        // Replace parameters in step prompts and other fields
        for step in &mut workflow.steps {
            // Replace in step name
            step.name = Self::replace_parameters(&step.name, &param_values);

            // Replace in action prompts
            match &mut step.action {
                StepAction::AgentTask { prompt } => {
                    *prompt = Self::replace_parameters(prompt, &param_values);
                }
                StepAction::ManualApproval { approvers } => {
                    // Replace approver placeholders
                    for approver in approvers {
                        *approver = Self::replace_parameters(approver, &param_values);
                    }
                }
                StepAction::Script { command, args } => {
                    *command = Self::replace_parameters(command, &param_values);
                    for arg in args {
                        *arg = Self::replace_parameters(arg, &param_values);
                    }
                }
                StepAction::ToolSequence { .. } => {
                    // Could replace tool parameters if needed
                }
            }
        }

        Ok(workflow)
    }

    /// Validate a parameter value against its type
    fn validate_parameter(param: &TemplateParameter, value: &str) -> Result<()> {
        match param.param_type {
            TemplateParameterType::String => Ok(()),
            TemplateParameterType::Number => {
                value
                    .parse::<f64>()
                    .context(format!("Parameter '{}' must be a number", param.name))?;
                Ok(())
            }
            TemplateParameterType::Boolean => {
                value
                    .parse::<bool>()
                    .context(format!("Parameter '{}' must be true or false", param.name))?;
                Ok(())
            }
            TemplateParameterType::RoleId
            | TemplateParameterType::TeamId
            | TemplateParameterType::PersonId => {
                // Just check it's not empty
                if value.trim().is_empty() {
                    anyhow::bail!("Parameter '{}' cannot be empty", param.name);
                }
                Ok(())
            }
        }
    }

    /// Replace parameter placeholders in a string
    /// Placeholders are in the form {{param_name}}
    fn replace_parameters(text: &str, params: &HashMap<String, String>) -> String {
        let mut result = text.to_string();

        for (name, value) in params {
            let placeholder = format!("{{{{{}}}}}", name);
            result = result.replace(&placeholder, value);
        }

        result
    }

    /// Extract parameter names from a template string
    pub fn extract_parameters(text: &str) -> Vec<String> {
        let mut params = Vec::new();
        let mut chars = text.chars().peekable();
        let mut current_param = String::new();
        let mut in_param = false;

        while let Some(c) = chars.next() {
            if c == '{' && chars.peek() == Some(&'{') {
                chars.next(); // consume second brace
                in_param = true;
                current_param.clear();
            } else if c == '}' && chars.peek() == Some(&'}') && in_param {
                chars.next(); // consume second brace
                if !current_param.is_empty() {
                    params.push(current_param.trim().to_string());
                }
                in_param = false;
                current_param.clear();
            } else if in_param {
                current_param.push(c);
            }
        }

        params.sort();
        params.dedup();
        params
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{RoleId, StepId, StepSpec};
    use chrono::Utc;

    #[test]
    fn test_extract_parameters() {
        let text = "Analyze {{file_path}} and report to {{reviewer}}";
        let params = TemplateProcessor::extract_parameters(text);
        assert_eq!(params, vec!["file_path", "reviewer"]);
    }

    #[test]
    fn test_replace_parameters() {
        let text = "Analyze {{file_path}} and report to {{reviewer}}";
        let mut params = HashMap::new();
        params.insert("file_path".to_string(), "src/main.rs".to_string());
        params.insert("reviewer".to_string(), "alice".to_string());

        let result = TemplateProcessor::replace_parameters(text, &params);
        assert_eq!(result, "Analyze src/main.rs and report to alice");
    }

    #[test]
    fn test_instantiate_template() {
        let template = ProcessTemplate {
            id: TemplateId::new("code_review"),
            name: "Code Review".to_string(),
            description: "Standard code review process".to_string(),
            category: "code_review".to_string(),
            parameters: vec![
                TemplateParameter {
                    name: "file_path".to_string(),
                    description: "Path to the file to review".to_string(),
                    param_type: TemplateParameterType::String,
                    default_value: None,
                    required: true,
                },
                TemplateParameter {
                    name: "reviewer".to_string(),
                    description: "Person who will review".to_string(),
                    param_type: TemplateParameterType::PersonId,
                    default_value: Some("alice".to_string()),
                    required: false,
                },
            ],
            workflow_template: WorkflowSpec {
                steps: vec![StepSpec {
                    id: StepId::new("review"),
                    name: "Review {{file_path}}".to_string(),
                    description: Some("Review code file".to_string()),
                    role: RoleId::new("reviewer"),
                    action: StepAction::AgentTask {
                        prompt: "Review the code in {{file_path}} and report to {{reviewer}}"
                            .to_string(),
                    },
                    timeout_secs: None,
                    retry_policy: None,
                    requires_approval: false,
                }],
                dependencies: HashMap::new(),
            },
            created_at: Utc::now(),
            created_by: "admin".to_string(),
        };

        let mut params = HashMap::new();
        params.insert("file_path".to_string(), "src/main.rs".to_string());

        let instance = TemplateInstance {
            template_id: TemplateId::new("code_review"),
            parameters: params,
            created_at: Utc::now(),
            created_by: "user".to_string(),
        };

        let workflow = TemplateProcessor::instantiate(&template, &instance).unwrap();

        assert_eq!(workflow.steps[0].name, "Review src/main.rs");
        if let StepAction::AgentTask { prompt } = &workflow.steps[0].action {
            assert_eq!(prompt, "Review the code in src/main.rs and report to alice");
        } else {
            panic!("Expected AgentTask");
        }
    }

    #[test]
    fn test_missing_required_parameter() {
        let template = ProcessTemplate {
            id: TemplateId::new("test"),
            name: "Test".to_string(),
            description: "Test template".to_string(),
            category: "test".to_string(),
            parameters: vec![TemplateParameter {
                name: "required_param".to_string(),
                description: "A required parameter".to_string(),
                param_type: TemplateParameterType::String,
                default_value: None,
                required: true,
            }],
            workflow_template: WorkflowSpec {
                steps: vec![],
                dependencies: HashMap::new(),
            },
            created_at: Utc::now(),
            created_by: "admin".to_string(),
        };

        let instance = TemplateInstance {
            template_id: TemplateId::new("test"),
            parameters: HashMap::new(),
            created_at: Utc::now(),
            created_by: "user".to_string(),
        };

        let result = TemplateProcessor::instantiate(&template, &instance);
        assert!(result.is_err());
    }

    #[test]
    fn test_parameter_type_validation() {
        let param = TemplateParameter {
            name: "count".to_string(),
            description: "A number".to_string(),
            param_type: TemplateParameterType::Number,
            default_value: None,
            required: true,
        };

        assert!(TemplateProcessor::validate_parameter(&param, "42").is_ok());
        assert!(TemplateProcessor::validate_parameter(&param, "3.14").is_ok());
        assert!(TemplateProcessor::validate_parameter(&param, "not a number").is_err());
    }
}
