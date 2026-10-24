use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// What a run is asked to do, stored as the `spec` of its `RunCreated` event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunSpec {
    pub task: String,
    pub agent: AgentSpec,
    #[serde(default)]
    pub runtime: Runtime,
}

/// The agent working on a run. Inline for now; versioned agent definitions
/// come later (`docs/design/runtimes-and-providers.md`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentSpec {
    /// For the native runtime: the provider model. For a harness: passed
    /// through as its `--model`, or the harness's default when empty.
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub system: String,
    /// Native runtime: names of Shiioo tools the agent may use. Harness:
    /// the harness's own tool allow-list syntax (for example
    /// `Bash(git:*)`).
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
}

fn default_max_turns() -> u32 {
    50
}

/// What executes the run (`docs/adr/0009-harness-first-runtimes.md`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Runtime {
    /// Shiioo's own loop: a model provider plus Shiioo tools on the machine.
    #[default]
    Native,
    /// An external harness driven headless on the machine.
    Harness(HarnessSpec),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HarnessSpec {
    pub harness: HarnessKind,
    /// A git repository to work on. The run gets its own worktree and
    /// branch (`shiioo/<run>`); without a repo it works in the machine's
    /// workspace directly.
    #[serde(default)]
    pub repo: Option<PathBuf>,
    /// The harness's permission mode when no human is attached
    /// (Claude Code: `default`, `acceptEdits`, `plan`, `bypassPermissions`).
    #[serde(default = "default_permission_mode")]
    pub permission_mode: String,
    /// Which of the harness's setting sources apply (Claude Code:
    /// `user`, `project`, `local`). The owner's personal hooks and MCP
    /// servers stay out of unattended runs by default.
    #[serde(default = "default_setting_sources")]
    pub setting_sources: String,
    /// Stop the harness once it has spent this much (API billing only;
    /// subscriptions report zero cost).
    #[serde(default)]
    pub max_budget_usd: Option<f64>,
    /// What the harness bills. In headless mode Claude Code prefers an
    /// `ANTHROPIC_API_KEY` in its environment over the owner's login, so
    /// subscription runs get that variable removed.
    #[serde(default)]
    pub billing: HarnessBilling,
    /// Path of the harness executable. Default: looked up on `PATH`
    /// (`claude`), or `SHIIOO_CLAUDE_BIN`.
    #[serde(default)]
    pub binary: Option<PathBuf>,
}

impl HarnessSpec {
    /// Claude Code with the defaults: subscription billing, `acceptEdits`,
    /// project settings only, no repo.
    pub fn claude_code() -> Self {
        Self {
            harness: HarnessKind::ClaudeCode,
            repo: None,
            permission_mode: default_permission_mode(),
            setting_sources: default_setting_sources(),
            max_budget_usd: None,
            billing: HarnessBilling::default(),
            binary: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessBilling {
    /// The owner's subscription (Claude Max, …), via the harness's login.
    #[default]
    Subscription,
    /// API credits, via `ANTHROPIC_API_KEY` in the environment.
    ApiKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessKind {
    ClaudeCode,
}

impl HarnessKind {
    pub fn name(self) -> &'static str {
        match self {
            HarnessKind::ClaudeCode => "claude-code",
        }
    }
}

fn default_permission_mode() -> String {
    "acceptEdits".into()
}

fn default_setting_sources() -> String {
    "project".into()
}
