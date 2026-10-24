//! Claude Code driven headless: `claude -p … --output-format stream-json`.
//!
//! Each stdout line is one JSON object. The shapes below were recorded from
//! Claude Code 2.1.310 (`tests/fixtures/claude-code-read.jsonl`):
//!
//! - `{"type":"system","subtype":"init","session_id","cwd","model","tools",
//!   "claude_code_version",…}` once at the start;
//! - `{"type":"assistant","message":{<Messages API message>},…}` per model
//!   response, `content` holding `text` and `tool_use` blocks;
//! - `{"type":"user","message":{"content":[{"type":"tool_result",
//!   "tool_use_id","content","is_error"?}]},…}` per batch of tool results;
//! - `{"type":"rate_limit_event","rate_limit_info":{"unifiedWindows":…}}`;
//! - `{"type":"result","subtype":"success"|"error_…","is_error","result",
//!   "session_id","total_cost_usd","usage","permission_denials",…}` last.

use crate::spec::{AgentSpec, HarnessBilling, HarnessSpec};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

pub const HARNESS: &str = "claude-code";

/// Variables that identify the Claude Code session Shiioo itself may be
/// running inside. The child must not inherit them. Credentials and
/// config (`CLAUDE_CODE_OAUTH_TOKEN`, `CLAUDE_CONFIG_DIR`) are kept.
const SESSION_ENV: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
];

/// Prompt for resuming a session whose process was stopped mid-task.
pub const CONTINUE_PROMPT: &str = "The process running you was stopped before you finished. \
    Check the current state of the workspace, then continue the task to completion.";

/// Builds the command for a new session, or for resuming `resume` with a
/// follow-up prompt.
pub fn command(
    binary: &Path,
    spec: &HarnessSpec,
    agent: &AgentSpec,
    cwd: &Path,
    prompt: &str,
    resume: Option<&str>,
) -> Command {
    let (model, system, tools, max_turns) = (
        agent.model.as_str(),
        agent.system.as_str(),
        agent.tools.as_slice(),
        agent.max_turns,
    );
    let mut cmd = Command::new(binary);
    // The prompt goes right after --print: later options take variable
    // numbers of values and would swallow a trailing positional.
    cmd.current_dir(cwd)
        .arg("--print")
        .arg(prompt)
        .arg("--output-format")
        .arg("stream-json")
        .arg("--verbose")
        .arg("--permission-mode")
        .arg(&spec.permission_mode)
        .arg("--setting-sources")
        .arg(&spec.setting_sources)
        .arg("--max-turns")
        .arg(max_turns.to_string());
    if !model.is_empty() {
        cmd.arg("--model").arg(model);
    }
    if !system.is_empty() {
        cmd.arg("--append-system-prompt").arg(system);
    }
    if !tools.is_empty() {
        cmd.arg("--allowedTools");
        cmd.args(tools);
    }
    if let Some(budget) = spec.max_budget_usd {
        cmd.arg("--max-budget-usd").arg(budget.to_string());
    }
    if let Some(session) = resume {
        cmd.arg("--resume")
            .arg(session)
            .env("CLAUDE_CODE_RESUME_INTERRUPTED_TURN", "1");
    }
    // A Claude Code started from inside another one (as Shiioo itself may
    // be) must not inherit the parent session's identity.
    for key in SESSION_ENV {
        cmd.env_remove(key);
    }
    if spec.billing == HarnessBilling::Subscription {
        cmd.env_remove("ANTHROPIC_API_KEY");
        cmd.env_remove("ANTHROPIC_AUTH_TOKEN");
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd
}

/// The binary to run: the spec's, else `SHIIOO_CLAUDE_BIN`, else `claude`
/// on the path.
pub fn binary(spec: &HarnessSpec) -> PathBuf {
    spec.binary.clone().unwrap_or_else(|| {
        std::env::var_os("SHIIOO_CLAUDE_BIN").map_or_else(|| PathBuf::from("claude"), PathBuf::from)
    })
}

/// One parsed stdout line.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    Init {
        session_id: String,
        model: String,
        cwd: String,
        version: String,
    },
    /// A model response, verbatim (`message` of the assistant line).
    Assistant {
        message: Value,
    },
    ToolResults(Vec<ToolResultLine>),
    RateLimit {
        windows: Value,
    },
    Result(ResultLine),
    Other(Value),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolResultLine {
    pub call_id: String,
    pub content: String,
    pub is_error: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResultLine {
    pub subtype: String,
    pub is_error: bool,
    pub text: String,
    /// Error subtypes carry these instead of `result`.
    pub errors: Vec<String>,
    pub session_id: String,
    pub cost_usd: Option<f64>,
    pub usage: Value,
    pub permission_denials: Value,
}

pub fn parse_line(line: &str) -> Result<Line, serde_json::Error> {
    let v: Value = serde_json::from_str(line)?;
    let str_field = |v: &Value, key: &str| v[key].as_str().unwrap_or_default().to_owned();
    Ok(match (v["type"].as_str(), v["subtype"].as_str()) {
        (Some("system"), Some("init")) => Line::Init {
            session_id: str_field(&v, "session_id"),
            model: str_field(&v, "model"),
            cwd: str_field(&v, "cwd"),
            version: str_field(&v, "claude_code_version"),
        },
        (Some("assistant"), _) => Line::Assistant {
            message: v["message"].clone(),
        },
        (Some("user"), _) => Line::ToolResults(
            v["message"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|b| b["type"] == "tool_result")
                .map(|b| ToolResultLine {
                    call_id: str_field(b, "tool_use_id"),
                    content: content_text(&b["content"]),
                    is_error: b["is_error"].as_bool().unwrap_or(false),
                })
                .collect(),
        ),
        (Some("rate_limit_event"), _) => Line::RateLimit {
            windows: v["rate_limit_info"]["unifiedWindows"].clone(),
        },
        (Some("result"), _) => Line::Result(ResultLine {
            subtype: str_field(&v, "subtype"),
            is_error: v["is_error"].as_bool().unwrap_or(false),
            text: str_field(&v, "result"),
            errors: v["errors"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| e.as_str().map(str::to_owned))
                .collect(),
            session_id: str_field(&v, "session_id"),
            cost_usd: v["total_cost_usd"].as_f64(),
            usage: v["usage"].clone(),
            permission_denials: v["permission_denials"].clone(),
        }),
        _ => Line::Other(v),
    })
}

/// Tool result content is a string or an array of content blocks.
fn content_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../tests/fixtures/claude-code-read.jsonl");

    #[test]
    fn parses_the_recorded_session() {
        let lines: Vec<Line> = FIXTURE
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| parse_line(l).unwrap_or_else(|e| panic!("{e}: {l}")))
            .collect();

        let Line::Init {
            session_id, model, ..
        } = &lines[0]
        else {
            panic!("first line is not init: {:?}", lines[0]);
        };
        assert_eq!(session_id, "a11a080d-e85c-457e-9b31-578ebfba0c1c");
        assert_eq!(model, "claude-haiku-5-5");

        let Line::Assistant { message } = &lines[1] else {
            panic!("{:?}", lines[1]);
        };
        assert_eq!(message["content"][0]["type"], "tool_use");
        assert_eq!(message["content"][0]["name"], "Read");

        let Line::ToolResults(results) = &lines[2] else {
            panic!("{:?}", lines[2]);
        };
        assert_eq!(results[0].call_id, "toolu_01HMiigSvY68ECQqxyVaSd1q");
        assert!(results[0].content.starts_with("1\tfirst line"));
        assert!(!results[0].is_error);

        let Line::RateLimit { windows } = &lines[3] else {
            panic!("{:?}", lines[3]);
        };
        assert_eq!(windows["seven_day"]["utilization"], 0.92);

        let Line::Result(result) = lines.last().unwrap_or(&Line::Other(Value::Null)) else {
            panic!("last line is not a result");
        };
        assert_eq!(result.subtype, "success");
        assert!(!result.is_error);
        assert_eq!(result.text, "first line of readme");
        assert_eq!(result.session_id, "a11a080d-e85c-457e-9b31-578ebfba0c1c");
        assert!(result.cost_usd.is_some_and(|c| c > 0.0));
    }

    #[test]
    fn command_scrubs_claude_env_and_passes_flags() {
        let spec = HarnessSpec {
            harness: crate::spec::HarnessKind::ClaudeCode,
            repo: None,
            permission_mode: "acceptEdits".into(),
            setting_sources: "project".into(),
            max_budget_usd: Some(2.5),
            billing: HarnessBilling::Subscription,
            binary: None,
        };
        let agent = AgentSpec {
            model: "claude-opus-5-5".into(),
            system: "Be brief.".into(),
            tools: vec!["Read".into(), "Bash(git:*)".into()],
            max_turns: 20,
        };
        let cmd = command(
            Path::new("claude"),
            &spec,
            &agent,
            Path::new("/tmp"),
            "do it",
            Some("sess-1"),
        );
        let std_cmd = cmd.as_std();
        let args: Vec<String> = std_cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        for expected in [
            "--print",
            "stream-json",
            "--verbose",
            "acceptEdits",
            "--max-turns",
            "20",
            "claude-opus-5-5",
            "Be brief.",
            "--allowedTools",
            "Bash(git:*)",
            "--max-budget-usd",
            "2.5",
            "--resume",
            "sess-1",
            "do it",
        ] {
            assert!(
                args.iter().any(|a| a == expected),
                "missing {expected}: {args:?}"
            );
        }
        // The prompt comes before the variadic --allowedTools.
        let prompt_at = args.iter().position(|a| a == "do it");
        let tools_at = args.iter().position(|a| a == "--allowedTools");
        assert!(prompt_at < tools_at, "{args:?}");
        let removed: Vec<String> = std_cmd
            .get_envs()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect();
        for key in SESSION_ENV {
            assert!(removed.iter().any(|r| r == key), "{key} not scrubbed");
        }
        assert!(removed.iter().any(|r| r == "ANTHROPIC_API_KEY"));
        assert!(!removed.iter().any(|r| r == "CLAUDE_CODE_OAUTH_TOKEN"));
        let set: Vec<String> = std_cmd
            .get_envs()
            .filter(|(_, v)| v.is_some())
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect();
        assert!(
            set.iter()
                .any(|k| k == "CLAUDE_CODE_RESUME_INTERRUPTED_TURN")
        );
    }
}
