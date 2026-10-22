use serde::{Deserialize, Serialize};

/// What a run is asked to do, stored as the `spec` of its `RunCreated` event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunSpec {
    pub task: String,
    pub agent: AgentSpec,
}

/// The agent working on a run. Inline for now; versioned agent definitions
/// come later (`docs/design/runtimes-and-providers.md`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentSpec {
    pub model: String,
    #[serde(default)]
    pub system: String,
    /// Names of the tools the agent may use.
    pub tools: Vec<String>,
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
}

fn default_max_turns() -> u32 {
    50
}
