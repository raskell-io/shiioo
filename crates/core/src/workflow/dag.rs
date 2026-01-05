use crate::types::{StepId, StepSpec, WorkflowSpec};
use anyhow::{anyhow, Context, Result};
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::Topo;
use std::collections::HashMap;

/// DAG representation of a workflow
pub struct WorkflowDag {
    graph: DiGraph<StepSpec, ()>,
    step_indices: HashMap<StepId, NodeIndex>,
}

impl WorkflowDag {
    /// Build a DAG from a workflow specification
    pub fn from_workflow(workflow: &WorkflowSpec) -> Result<Self> {
        let mut graph = DiGraph::new();
        let mut step_indices = HashMap::new();

        // Add all steps as nodes
        for step in &workflow.steps {
            let node = graph.add_node(step.clone());
            step_indices.insert(step.id.clone(), node);
        }

        // Add dependency edges
        for (step_id, dependencies) in &workflow.dependencies {
            let step_idx = step_indices
                .get(step_id)
                .ok_or_else(|| anyhow!("Step {} referenced in dependencies but not defined", step_id))?;

            for dep_id in dependencies {
                let dep_idx = step_indices
                    .get(dep_id)
                    .ok_or_else(|| anyhow!("Dependency {} not found for step {}", dep_id, step_id))?;

                // Edge from dependency to dependent (dep -> step)
                graph.add_edge(*dep_idx, *step_idx, ());
            }
        }

        // Verify the graph is acyclic
        if petgraph::algo::is_cyclic_directed(&graph) {
            return Err(anyhow!("Workflow contains circular dependencies"));
        }

        Ok(Self {
            graph,
            step_indices,
        })
    }

    /// Get steps in topological order (dependencies first)
    pub fn topological_order(&self) -> Vec<StepSpec> {
        let mut topo = Topo::new(&self.graph);
        let mut steps = Vec::new();

        while let Some(node) = topo.next(&self.graph) {
            steps.push(self.graph[node].clone());
        }

        steps
    }

    /// Get dependencies for a step
    pub fn dependencies(&self, step_id: &StepId) -> Result<Vec<StepId>> {
        let node = self
            .step_indices
            .get(step_id)
            .ok_or_else(|| anyhow!("Step {} not found", step_id))?;

        let deps: Vec<StepId> = self
            .graph
            .neighbors_directed(*node, petgraph::Direction::Incoming)
            .map(|n| self.graph[n].id.clone())
            .collect();

        Ok(deps)
    }

    /// Get steps that depend on the given step
    pub fn dependents(&self, step_id: &StepId) -> Result<Vec<StepId>> {
        let node = self
            .step_indices
            .get(step_id)
            .ok_or_else(|| anyhow!("Step {} not found", step_id))?;

        let deps: Vec<StepId> = self
            .graph
            .neighbors_directed(*node, petgraph::Direction::Outgoing)
            .map(|n| self.graph[n].id.clone())
            .collect();

        Ok(deps)
    }

    /// Check if all dependencies of a step are satisfied
    pub fn can_execute(
        &self,
        step_id: &StepId,
        completed_steps: &std::collections::HashSet<StepId>,
    ) -> Result<bool> {
        let deps = self.dependencies(step_id)?;
        Ok(deps.iter().all(|dep| completed_steps.contains(dep)))
    }

    /// Get all steps with no dependencies (can start immediately)
    pub fn entry_steps(&self) -> Vec<StepSpec> {
        self.graph
            .node_indices()
            .filter(|&n| {
                self.graph
                    .neighbors_directed(n, petgraph::Direction::Incoming)
                    .count()
                    == 0
            })
            .map(|n| self.graph[n].clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{RoleId, StepAction};

    fn create_test_step(id: &str, name: &str) -> StepSpec {
        StepSpec {
            id: StepId::new(id),
            name: name.to_string(),
            description: None,
            role: RoleId::new("test-role"),
            action: StepAction::AgentTask {
                prompt: "test".to_string(),
            },
            timeout_secs: None,
            retry_policy: None,
            requires_approval: false,
        }
    }

    #[test]
    fn test_linear_dag() {
        let workflow = WorkflowSpec {
            steps: vec![
                create_test_step("step1", "Step 1"),
                create_test_step("step2", "Step 2"),
                create_test_step("step3", "Step 3"),
            ],
            dependencies: [
                (StepId::new("step2"), vec![StepId::new("step1")]),
                (StepId::new("step3"), vec![StepId::new("step2")]),
            ]
            .iter()
            .cloned()
            .collect(),
        };

        let dag = WorkflowDag::from_workflow(&workflow).unwrap();
        let order = dag.topological_order();

        assert_eq!(order.len(), 3);
        assert_eq!(order[0].id.0, "step1");
        assert_eq!(order[1].id.0, "step2");
        assert_eq!(order[2].id.0, "step3");
    }

    #[test]
    fn test_parallel_dag() {
        let workflow = WorkflowSpec {
            steps: vec![
                create_test_step("step1", "Step 1"),
                create_test_step("step2", "Step 2"),
                create_test_step("step3", "Step 3"),
                create_test_step("step4", "Step 4"),
            ],
            dependencies: [
                (StepId::new("step3"), vec![StepId::new("step1")]),
                (StepId::new("step3"), vec![StepId::new("step2")]),
                (StepId::new("step4"), vec![StepId::new("step3")]),
            ]
            .iter()
            .cloned()
            .collect(),
        };

        let dag = WorkflowDag::from_workflow(&workflow).unwrap();
        let entry = dag.entry_steps();

        // step1 and step2 can start in parallel
        assert_eq!(entry.len(), 2);

        let deps = dag.dependencies(&StepId::new("step3")).unwrap();
        assert_eq!(deps.len(), 2);
    }

    #[test]
    fn test_cyclic_dag_rejected() {
        let workflow = WorkflowSpec {
            steps: vec![
                create_test_step("step1", "Step 1"),
                create_test_step("step2", "Step 2"),
            ],
            dependencies: [
                (StepId::new("step1"), vec![StepId::new("step2")]),
                (StepId::new("step2"), vec![StepId::new("step1")]),
            ]
            .iter()
            .cloned()
            .collect(),
        };

        let result = WorkflowDag::from_workflow(&workflow);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("circular dependencies"));
    }
}
