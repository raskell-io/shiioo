//! Codex driven headless: `codex exec --json …`.
//!
//! Each stdout line is one JSON object. Shapes recorded from Codex CLI
//! 0.163.0 (`tests/fixtures/codex-command.jsonl`, `codex-edit.jsonl`):
//!
//! - `{"type":"thread.started","thread_id":…}` once at the start;
//! - `{"type":"turn.started"}`;
//! - `{"type":"item.started"|"item.completed","item":{"id","type",…}}`, with
//!   item types `agent_message {text}`, `reasoning {text}`,
//!   `command_execution {command, aggregated_output, exit_code, status}`,
//!   `file_change {changes}`, `mcp_tool_call`, `web_search`, `todo_list`,
//!   `error {message}`;
//! - `{"type":"turn.completed","usage":{input_tokens, cached_input_tokens,
//!   cache_write_input_tokens, output_tokens, reasoning_output_tokens}}`
//!   or `{"type":"turn.failed","error":{…}}` last.
//!
//! Resume: `codex exec resume <thread_id> --json <prompt>`.

use crate::harness::claude_code::HookEndpoint;
use crate::spec::{AgentSpec, HarnessBilling, HarnessSpec};
use crate::tools::digest_hex;
use serde_json::Value;
use shiioo_core::ToolClass;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

pub const HARNESS: &str = "codex";

/// Codex's three sandbox policies for the commands the agent runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sandbox {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

/// Maps Shiioo's (Claude Code flavoured) permission modes onto Codex's
/// sandbox; Codex's own names are accepted too.
pub fn sandbox_for(permission_mode: &str) -> Sandbox {
    match permission_mode {
        "default" | "plan" | "read-only" => Sandbox::ReadOnly,
        "bypassPermissions" | "danger-full-access" => Sandbox::DangerFullAccess,
        _ => Sandbox::WorkspaceWrite,
    }
}

impl Sandbox {
    fn config_value(self) -> &'static str {
        match self {
            Sandbox::ReadOnly => "read-only",
            Sandbox::WorkspaceWrite => "workspace-write",
            Sandbox::DangerFullAccess => "danger-full-access",
        }
    }
}

/// The `-c hooks.PreToolUse=…` value installing Shiioo's permission hook:
/// the same `shiioo hook pre-tool-use` program Claude Code calls, because
/// Codex's hook contract (stdin `tool_name`/`tool_input`/`tool_use_id`,
/// stdout `hookSpecificOutput.permissionDecision`) is the same.
pub fn hook_config(exe: &Path) -> String {
    let command = exe
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!(
        "hooks.PreToolUse=[{{matcher=\"^(Bash|shell)$\",hooks=[{{type=\"command\",\
         command=\"{command} hook pre-tool-use\",timeout={}}}]}}]",
        crate::harness::claude_code::HOOK_TIMEOUT_SECS
    )
}

/// Builds `codex exec …` for a new thread, or `codex exec resume <thread>`
/// with a follow-up prompt.
///
/// `approval_policy=never`: in print mode nobody can answer a prompt, so a
/// command the sandbox refuses is refused, not escalated. Shiioo's rules
/// and the owner decide through the hook instead, when `hooks` is given.
#[allow(clippy::too_many_arguments)]
pub fn command(
    binary: &Path,
    spec: &HarnessSpec,
    agent: &AgentSpec,
    cwd: &Path,
    prompt: &str,
    resume: Option<&str>,
    hooks: Option<(&HookEndpoint, &str)>,
) -> Command {
    let mut cmd = Command::new(binary);
    cmd.args(&spec.binary_args);
    cmd.current_dir(cwd).arg("exec");
    if let Some(thread) = resume {
        cmd.arg("resume").arg(thread);
    }
    cmd.arg("--json")
        .arg("-C")
        .arg(cwd)
        .arg("-c")
        .arg("approval_policy=\"never\"");
    match sandbox_for(&spec.permission_mode) {
        Sandbox::DangerFullAccess => {
            cmd.arg("--dangerously-bypass-approvals-and-sandbox");
        }
        sandbox => {
            cmd.arg("-c")
                .arg(format!("sandbox_mode=\"{}\"", sandbox.config_value()));
        }
    }
    if !agent.model.is_empty() {
        cmd.arg("-m").arg(&agent.model);
    }
    if spec.repo.is_none() {
        cmd.arg("--skip-git-repo-check");
    }
    if let Some((endpoint, run_id)) = hooks {
        cmd.arg("-c")
            .arg("features.hooks=true")
            .arg("-c")
            .arg(hook_config(&endpoint.exe))
            .arg("--dangerously-bypass-hook-trust")
            .env("SHIIOO_SERVER_URL", &endpoint.server_url)
            .env("SHIIOO_API_TOKEN", &endpoint.token)
            .env("SHIIOO_RUN_ID", run_id);
    }
    // Codex has no system-prompt flag; extra instructions ride along with
    // the task. The prompt is the last argument.
    let full_prompt = if agent.system.is_empty() {
        prompt.to_owned()
    } else {
        format!("Instructions:\n{}\n\nTask:\n{prompt}", agent.system)
    };
    cmd.arg(full_prompt);
    if spec.billing == HarnessBilling::Subscription {
        // Keys in the environment would be used instead of the login.
        cmd.env_remove("OPENAI_API_KEY");
        cmd.env_remove("CODEX_API_KEY");
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd
}

/// The binary to run: the spec's, else `SHIIOO_CODEX_BIN`, else `codex`
/// on the path.
pub fn binary(spec: &HarnessSpec) -> PathBuf {
    spec.binary.clone().unwrap_or_else(|| {
        std::env::var_os("SHIIOO_CODEX_BIN").map_or_else(|| PathBuf::from("codex"), PathBuf::from)
    })
}

/// `codex --version` → "0.163.0"; empty if the binary doesn't say.
pub async fn version(binary: &Path) -> String {
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        Command::new(binary).arg("--version").output(),
    )
    .await;
    match out {
        Ok(Ok(out)) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .last()
            .unwrap_or_default()
            .to_owned(),
        _ => String::new(),
    }
}

/// One parsed stdout line.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    ThreadStarted {
        thread_id: String,
    },
    TurnStarted,
    ItemStarted(Item),
    ItemCompleted(Item),
    TurnCompleted {
        usage: Value,
    },
    TurnFailed {
        message: String,
    },
    /// A top-level `error` event.
    Error {
        message: String,
    },
    Other(Value),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub kind: ItemKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    AgentMessage {
        text: String,
    },
    Reasoning,
    CommandExecution {
        command: String,
        aggregated_output: String,
        exit_code: Option<i64>,
        status: String,
    },
    FileChange {
        changes: Value,
    },
    McpToolCall {
        server: String,
        tool: String,
        arguments: Value,
        result: Value,
    },
    WebSearch {
        query: String,
    },
    TodoList,
    Error {
        message: String,
    },
    Unknown(String),
}

pub fn parse_line(line: &str) -> Result<Line, serde_json::Error> {
    let v: Value = serde_json::from_str(line)?;
    let s = |v: &Value, key: &str| v[key].as_str().unwrap_or_default().to_owned();
    Ok(match v["type"].as_str() {
        Some("thread.started") => Line::ThreadStarted {
            thread_id: s(&v, "thread_id"),
        },
        Some("turn.started") => Line::TurnStarted,
        Some("item.started") => Line::ItemStarted(parse_item(&v["item"])),
        Some("item.completed") => Line::ItemCompleted(parse_item(&v["item"])),
        Some("turn.completed") => Line::TurnCompleted {
            usage: v["usage"].clone(),
        },
        Some("turn.failed") => Line::TurnFailed {
            message: error_message(&v["error"]),
        },
        Some("error") => Line::Error {
            message: error_message(&v),
        },
        _ => Line::Other(v),
    })
}

fn error_message(v: &Value) -> String {
    v["message"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| v.to_string())
}

fn parse_item(item: &Value) -> Item {
    let s = |key: &str| item[key].as_str().unwrap_or_default().to_owned();
    let kind = match item["type"].as_str().unwrap_or_default() {
        "agent_message" => ItemKind::AgentMessage { text: s("text") },
        "reasoning" => ItemKind::Reasoning,
        "command_execution" => ItemKind::CommandExecution {
            command: s("command"),
            aggregated_output: s("aggregated_output"),
            exit_code: item["exit_code"].as_i64(),
            status: s("status"),
        },
        "file_change" => ItemKind::FileChange {
            changes: item["changes"].clone(),
        },
        "mcp_tool_call" => ItemKind::McpToolCall {
            server: s("server"),
            tool: s("tool"),
            arguments: item["arguments"].clone(),
            result: item["result"].clone(),
        },
        "web_search" => ItemKind::WebSearch { query: s("query") },
        "todo_list" => ItemKind::TodoList,
        "error" => ItemKind::Error {
            message: s("message"),
        },
        other => ItemKind::Unknown(other.to_owned()),
    };
    Item { id: s("id"), kind }
}

/// Codex applies edits itself; the intent's class says what re-applying
/// the same patch would mean.
pub fn file_change_class(changes: &Value) -> ToolClass {
    ToolClass::Idempotent {
        key: digest_hex(&[b"codex-file-change", changes.to_string().as_bytes()]),
    }
}

/// A one-line summary of a `file_change` item's changes.
pub fn file_change_summary(changes: &Value) -> String {
    let parts: Vec<String> = changes
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            format!(
                "{} {}",
                c["kind"].as_str().unwrap_or("changed"),
                c["path"].as_str().unwrap_or_default()
            )
        })
        .collect();
    if parts.is_empty() {
        "applied changes".to_owned()
    } else {
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMAND: &str = include_str!("../../tests/fixtures/codex-command.jsonl");
    const EDIT: &str = include_str!("../../tests/fixtures/codex-edit.jsonl");

    fn lines(fixture: &str) -> Vec<Line> {
        fixture
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| parse_line(l).unwrap_or_else(|e| panic!("{e}: {l}")))
            .collect()
    }

    #[test]
    fn parses_the_recorded_command_session() {
        let lines = lines(COMMAND);
        assert_eq!(
            lines[0],
            Line::ThreadStarted {
                thread_id: "01a17237-e570-7033-8077-8e667f293b33".into()
            }
        );
        assert_eq!(lines[1], Line::TurnStarted);
        let Line::ItemCompleted(Item {
            kind: ItemKind::AgentMessage { text },
            ..
        }) = &lines[2]
        else {
            panic!("{:?}", lines[2]);
        };
        assert!(text.starts_with("I’ll run"));
        let Line::ItemStarted(Item {
            id,
            kind:
                ItemKind::CommandExecution {
                    command,
                    exit_code,
                    status,
                    ..
                },
        }) = &lines[3]
        else {
            panic!("{:?}", lines[3]);
        };
        assert_eq!(id, "item_1");
        assert_eq!(command, "/bin/zsh -lc date");
        assert_eq!(*exit_code, None);
        assert_eq!(status, "in_progress");
        let Line::ItemCompleted(Item {
            kind:
                ItemKind::CommandExecution {
                    aggregated_output,
                    exit_code,
                    ..
                },
            ..
        }) = &lines[4]
        else {
            panic!("{:?}", lines[4]);
        };
        assert_eq!(aggregated_output, "Sun Oct 25 09:40:34 CET 2026\n");
        assert_eq!(*exit_code, Some(0));
        let Line::TurnCompleted { usage } = &lines[6] else {
            panic!("{:?}", lines[6]);
        };
        assert_eq!(usage["output_tokens"], 64);
    }

    #[test]
    fn parses_the_recorded_edit_session() {
        let lines = lines(EDIT);
        assert_eq!(lines.len(), 7);
        assert!(matches!(lines.last(), Some(Line::TurnCompleted { .. })));
    }

    #[test]
    fn command_builds_exec_and_resume_invocations() {
        let mut spec = HarnessSpec::codex();
        spec.repo = Some(PathBuf::from("/repo"));
        let agent = AgentSpec {
            model: "gpt-6-astra".into(),
            system: "Be brief.".into(),
            tools: vec![],
            max_turns: 5,
        };
        let cmd = command(
            Path::new("codex"),
            &spec,
            &agent,
            Path::new("/work/wt"),
            "do it",
            None,
            None,
        );
        let args: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "exec");
        assert!(args.contains(&"--json".to_string()));
        // Default: Codex's own sandbox is off; Shiioo's rules govern.
        assert!(
            args.contains(&"--dangerously-bypass-approvals-and-sandbox".to_string())
                || args.contains(&"sandbox_mode=\"danger-full-access\"".to_string()),
            "{args:?}"
        );
        assert!(args.contains(&"approval_policy=\"never\"".to_string()));
        assert!(args.contains(&"gpt-6-astra".to_string()));
        assert!(!args.contains(&"--skip-git-repo-check".to_string()));
        let c_at = args.iter().position(|a| a == "-C");
        assert_eq!(
            c_at.and_then(|i| args.get(i + 1)).map(String::as_str),
            Some("/work/wt")
        );
        assert_eq!(
            args.last().map(String::as_str),
            Some("Instructions:\nBe brief.\n\nTask:\ndo it")
        );
        let removed: Vec<String> = cmd
            .as_std()
            .get_envs()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect();
        assert!(removed.contains(&"OPENAI_API_KEY".to_string()));

        spec.repo = None;
        spec.permission_mode = "default".into();
        let cmd = command(
            Path::new("codex"),
            &spec,
            &agent,
            Path::new("/work"),
            "go on",
            Some("thread-1"),
            None,
        );
        let args: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(&args[..3], ["exec", "resume", "thread-1"]);
        assert!(args.contains(&"--skip-git-repo-check".to_string()));
        assert!(args.contains(&"sandbox_mode=\"read-only\"".to_string()));
    }

    #[test]
    fn hooks_install_the_shiioo_hook_and_pass_its_environment() {
        let spec = HarnessSpec::codex();
        let agent = AgentSpec {
            model: String::new(),
            system: String::new(),
            tools: vec![],
            max_turns: 5,
        };
        let endpoint = HookEndpoint {
            server_url: "http://127.0.0.1:7878".into(),
            token: "tok".into(),
            exe: PathBuf::from("/opt/shiioo"),
        };
        let cmd = command(
            Path::new("codex"),
            &spec,
            &agent,
            Path::new("/work"),
            "go",
            None,
            Some((&endpoint, "run-1")),
        );
        let std_cmd = cmd.as_std();
        let args: Vec<String> = std_cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"features.hooks=true".to_string()));
        assert!(args.contains(&"--dangerously-bypass-hook-trust".to_string()));
        let hook = args
            .iter()
            .find(|a| a.starts_with("hooks.PreToolUse="))
            .cloned()
            .unwrap_or_default();
        assert!(hook.contains("/opt/shiioo hook pre-tool-use"), "{hook}");
        assert!(hook.contains("matcher=\"^(Bash|shell)$\""), "{hook}");
        let env: Vec<(String, String)> = std_cmd
            .get_envs()
            .filter_map(|(k, v)| {
                Some((
                    k.to_string_lossy().into_owned(),
                    v?.to_string_lossy().into_owned(),
                ))
            })
            .collect();
        assert!(env.contains(&("SHIIOO_RUN_ID".into(), "run-1".into())));
        assert!(env.contains(&("SHIIOO_API_TOKEN".into(), "tok".into())));
    }

    #[test]
    fn sandbox_mapping() {
        assert_eq!(sandbox_for("acceptEdits"), Sandbox::WorkspaceWrite);
        assert_eq!(sandbox_for("default"), Sandbox::ReadOnly);
        assert_eq!(sandbox_for("bypassPermissions"), Sandbox::DangerFullAccess);
        assert_eq!(sandbox_for("workspace-write"), Sandbox::WorkspaceWrite);
    }
}
