use serde::{Deserialize, Serialize};
use shiioo_core::{RunId, Seq};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// What a run is asked to do, stored as the `spec` of its `RunCreated` event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunSpec {
    pub task: String,
    pub agent: AgentSpec,
    #[serde(default)]
    pub runtime: Runtime,
    /// Where the run must execute: a node's name or a `label=value`
    /// selector. None: the office's own machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    /// Native runtime: when the agent answers without calling a tool, park
    /// the run for the owner's next message (`RunParked { input }`) instead
    /// of completing it. A conversation is a run that parks.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub park_on_answer: bool,
    /// A follow-up: the finished run this one continues, in its worktree
    /// and branch and, for a harness, its session (`docs/design/desk.md` R9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

impl RunSpec {
    /// The run whose worktree, branch and sandbox state this run uses:
    /// its own, or for a follow-up the first run of the chain.
    pub fn workspace_run(&self, run_id: RunId) -> RunId {
        self.origin.as_ref().map_or(run_id, |o| o.workspace)
    }
}

/// Where a follow-up run comes from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Origin {
    /// The finished run it continues.
    pub run: RunId,
    /// That run's last event when the follow-up was made.
    pub seq: Seq,
    /// The first run of the chain: its worktree and branch are shared.
    pub workspace: RunId,
    /// The harness session to resume, if the chain started one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
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
// A harness spec carries its isolation settings; the enum is built a few
// times per run, so its size does not matter.
#[allow(clippy::large_enum_variant)]
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
    /// The repository's label in the office's `[repos]`. A node resolves
    /// the label to its own clone; the path above is the office's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_label: Option<String>,
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
    /// (`claude` or `codex`), or `SHIIOO_CLAUDE_BIN` / `SHIIOO_CODEX_BIN`.
    #[serde(default)]
    pub binary: Option<PathBuf>,
    /// Time budget: the run fails once the harness has run this long in
    /// total across resumes. None = no limit.
    #[serde(default)]
    pub max_minutes: Option<u64>,
    /// Arguments placed before the harness's own, for running it through
    /// an interpreter (tests run fake harnesses as `/bin/sh <script>`:
    /// macOS stalls for tens of seconds on the first direct execution of
    /// a freshly written script). Empty in production.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub binary_args: Vec<String>,
    /// Where the harness executes: as the owner's user in the worktree
    /// (default), in an OCI container, or in an OpenShell sandbox
    /// (`docs/design/security.md` §3.4).
    #[serde(default)]
    pub isolation: Isolation,
    /// Environment for the harness process. A value is literal or
    /// `secret:<name>`, resolved from the office's secret store when the
    /// harness starts; the log holds the reference, never the value.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

/// The isolation level a harness runs at (`docs/design/tools-and-machines.md` §4).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "level", rename_all = "snake_case")]
pub enum Isolation {
    /// Own git worktree and branch; the owner's user, files and network.
    #[default]
    Worktree,
    /// An OCI container per run (Docker or Podman): read-only root, no
    /// capabilities, the worktree and the repo's `.git` mounted, a
    /// persistent home per run, egress through an allow-listing proxy.
    Container(ContainerSpec),
    /// An NVIDIA OpenShell sandbox per run, with a network policy generated
    /// from the egress rules. Experimental: needs the `openshell` CLI and a
    /// gateway; works on an uploaded copy of the worktree.
    Openshell(OpenshellSpec),
}

impl Isolation {
    pub fn level(&self) -> &'static str {
        match self {
            Isolation::Worktree => "worktree",
            Isolation::Container(_) => "container",
            Isolation::Openshell(_) => "openshell",
        }
    }

    /// The level by name with its defaults.
    pub fn named(level: &str) -> Option<Self> {
        match level {
            "worktree" => Some(Isolation::Worktree),
            "container" => Some(Isolation::Container(ContainerSpec::default())),
            "openshell" => Some(Isolation::Openshell(OpenshellSpec::default())),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ContainerSpec {
    /// Image with the harnesses installed. Empty: Shiioo's own image, built
    /// from its embedded Dockerfile on first use.
    pub image: String,
    /// `docker` (default) or `podman`.
    pub engine: String,
    /// Memory limit, Docker syntax (`8g`).
    pub memory: Option<String>,
    pub cpus: Option<f64>,
    pub egress: Egress,
    /// Host directories mounted at the same path inside the container,
    /// besides the worktree and the repository's `.git`.
    pub mounts: Vec<Mount>,
    /// SSH remote prefixes rewritten to `https://github.com/` inside the
    /// container, which has no SSH keys: for example `git@github-work:`.
    /// `git@github.com:` and `ssh://git@github.com/` are always rewritten.
    pub git_https: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mount {
    pub host: PathBuf,
    #[serde(default)]
    pub readonly: bool,
}

/// What a sandboxed run may reach on the network. `worktree` runs are not
/// restricted (there is nothing to enforce it with).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Egress {
    /// Only these hosts (shell patterns such as `*.github.com`), through
    /// the sandbox's proxy; TLS on port 443 only. The office's own API is
    /// always reachable, for the permission hook.
    Allowlist { hosts: Vec<String> },
    /// Unrestricted.
    Open,
}

impl Default for Egress {
    fn default() -> Self {
        Egress::Allowlist {
            hosts: Egress::default_hosts(),
        }
    }
}

impl Egress {
    /// What coding agents need: their model providers, GitHub, and the
    /// package registries of the common toolchains.
    pub fn default_hosts() -> Vec<String> {
        [
            "api.anthropic.com",
            "statsig.anthropic.com",
            "api.openai.com",
            "chatgpt.com",
            "auth.openai.com",
            "github.com",
            "*.github.com",
            "*.githubusercontent.com",
            "registry.npmjs.org",
            "crates.io",
            "*.crates.io",
            "pypi.org",
            "files.pythonhosted.org",
            "deb.debian.org",
            "security.debian.org",
            "proxy.golang.org",
            "sum.golang.org",
        ]
        .map(String::from)
        .to_vec()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenshellSpec {
    /// Sandbox image (`--from`); empty: the gateway's default image.
    pub image: String,
    pub cpus: Option<f64>,
    /// Kubernetes quantity syntax (`4Gi`).
    pub memory: Option<String>,
    pub egress: Egress,
}

impl HarnessSpec {
    /// Codex with the defaults. Its own OS sandbox is off
    /// (`danger-full-access`): under `approval_policy = never` Codex cannot
    /// escalate, and `workspace-write` makes the checkout's git directory
    /// read-only, so the agent could never commit. Shiioo's rules and the
    /// owner's approvals govern every command instead; OS isolation is the
    /// job of the isolation levels (`docs/positioning.md`).
    pub fn codex() -> Self {
        Self {
            harness: HarnessKind::Codex,
            permission_mode: "danger-full-access".into(),
            ..Self::claude_code()
        }
    }

    /// Claude Code with the defaults: subscription billing, `acceptEdits`,
    /// project settings only, no repo.
    pub fn claude_code() -> Self {
        Self {
            harness: HarnessKind::ClaudeCode,
            repo: None,
            repo_label: None,
            permission_mode: default_permission_mode(),
            setting_sources: default_setting_sources(),
            max_budget_usd: None,
            billing: HarnessBilling::default(),
            binary: None,
            max_minutes: None,
            binary_args: Vec::new(),
            isolation: Isolation::Worktree,
            env: BTreeMap::new(),
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
    Codex,
}

impl HarnessKind {
    pub fn name(self) -> &'static str {
        match self {
            HarnessKind::ClaudeCode => "claude-code",
            HarnessKind::Codex => "codex",
        }
    }
}

fn default_permission_mode() -> String {
    "acceptEdits".into()
}

fn default_setting_sources() -> String {
    "project".into()
}
