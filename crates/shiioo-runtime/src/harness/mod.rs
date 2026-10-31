//! External agent harnesses driven as subprocesses (ADR-0009).
//!
//! A [`HarnessDriver`] knows how to start one harness and how to turn each
//! line it prints into Shiioo events; the worker's loop is the same for
//! all of them (`Session::run_harness`).

pub mod claude_code;
pub mod codex;

use crate::spec::{AgentSpec, HarnessKind, HarnessSpec};
use crate::tools::digest_hex;
use crate::worker::RunOutcome;
use claude_code::HookEndpoint;
use serde_json::{Value, json};
use shiioo_core::{CallId, Payload, ToolClass};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::process::{Child, ChildStdout, Command};

/// Prompt for resuming a session whose process was stopped mid-task.
pub const CONTINUE_PROMPT: &str = "The process running you was stopped before you finished. \
    Check the current state of the workspace, then continue the task to completion.";

/// Leads the owner's message when they stopped the harness to say it
/// ("Now" on the desk): the step in progress may not have finished.
pub const INTERRUPTED_PROMPT: &str = "The owner stopped you mid-step to tell you this. \
    Your last step may have been cut short, so check the workspace before you rely on it, \
    then carry on with the task with their message in mind:";

/// What one harness output line means for the run's log.
#[derive(Debug, Default)]
pub struct Interpretation {
    pub payloads: Vec<Payload>,
    /// The harness started (or resumed) a session with this id.
    pub session_started: Option<String>,
    /// The run is over; `payloads` ends with the terminal event.
    pub outcome: Option<RunOutcome>,
}

pub trait HarnessDriver: Send {
    fn name(&self) -> &'static str;
    fn binary(&self, spec: &HarnessSpec) -> PathBuf;
    fn command(
        &self,
        spec: &HarnessSpec,
        agent: &AgentSpec,
        cwd: &Path,
        prompt: &str,
        resume: Option<&str>,
        hooks: Option<(&HookEndpoint, &str)>,
    ) -> Command;
    fn interpret(&mut self, line: &str) -> Result<Interpretation, HarnessError>;
}

/// The driver for a spec, ready to run in `cwd`.
pub async fn driver_for(
    spec: &HarnessSpec,
    agent: &AgentSpec,
    cwd: &Path,
) -> Box<dyn HarnessDriver> {
    match spec.harness {
        HarnessKind::ClaudeCode => Box::new(ClaudeCodeDriver::default()),
        HarnessKind::Codex => {
            let version = codex::version(&codex::binary(spec), &spec.binary_args).await;
            Box::new(CodexDriver::new(agent, cwd, version))
        }
    }
}

fn parse_error(e: impl std::fmt::Display, line: &str) -> HarnessError {
    HarnessError::Parse(format!("{e}: {}", truncate(line, 200)))
}

fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

fn text_of(message: &Value) -> String {
    message["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// How a Claude Code tool call behaves if its outcome goes unrecorded.
/// Observational for harness runs (Shiioo doesn't re-run them), but it
/// keeps the log's meaning uniform.
pub(crate) fn claude_tool_class(tool: &str, input: &Value) -> ToolClass {
    match tool {
        "Read" | "Glob" | "Grep" | "LS" | "WebFetch" | "WebSearch" | "NotebookRead"
        | "TodoRead" | "ToolSearch" | "LSP" => ToolClass::Pure,
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" | "TodoWrite" => ToolClass::Idempotent {
            key: digest_hex(&[tool.as_bytes(), input.to_string().as_bytes()]),
        },
        _ => ToolClass::SideEffecting,
    }
}

#[derive(Default)]
pub struct ClaudeCodeDriver {
    last_text: String,
}

impl HarnessDriver for ClaudeCodeDriver {
    fn name(&self) -> &'static str {
        claude_code::HARNESS
    }

    fn binary(&self, spec: &HarnessSpec) -> PathBuf {
        claude_code::binary(spec)
    }

    fn command(
        &self,
        spec: &HarnessSpec,
        agent: &AgentSpec,
        cwd: &Path,
        prompt: &str,
        resume: Option<&str>,
        hooks: Option<(&HookEndpoint, &str)>,
    ) -> Command {
        claude_code::command(&self.binary(spec), spec, agent, cwd, prompt, resume, hooks)
    }

    fn interpret(&mut self, line: &str) -> Result<Interpretation, HarnessError> {
        use claude_code::Line;
        let mut out = Interpretation::default();
        match claude_code::parse_line(line).map_err(|e| parse_error(e, line))? {
            Line::Init {
                session_id,
                model,
                cwd,
                version,
            } => {
                out.session_started = Some(session_id.clone());
                out.payloads.push(Payload::HarnessSessionStarted {
                    harness: claude_code::HARNESS.into(),
                    version,
                    session_id,
                    cwd,
                    model,
                });
            }
            Line::Assistant { message } => {
                let text = text_of(&message);
                if !text.is_empty() {
                    self.last_text = text;
                }
                out.payloads.push(Payload::ModelResponseRecorded {
                    model: message["model"].as_str().unwrap_or_default().to_owned(),
                    response: message.clone(),
                    context_seq: None,
                });
                for block in message["content"].as_array().into_iter().flatten() {
                    if block["type"] != "tool_use" {
                        continue;
                    }
                    let tool = block["name"].as_str().unwrap_or_default().to_owned();
                    let input = block["input"].clone();
                    out.payloads.push(Payload::ToolIntent {
                        call_id: CallId(block["id"].as_str().unwrap_or_default().to_owned()),
                        class: claude_tool_class(&tool, &input),
                        tool,
                        input,
                    });
                }
            }
            Line::ToolResults(results) => {
                out.payloads
                    .extend(results.into_iter().map(|r| Payload::ToolResult {
                        call_id: CallId(r.call_id),
                        output: json!(r.content),
                        is_error: r.is_error,
                    }));
            }
            Line::RateLimit { windows } => {
                out.payloads.push(Payload::CapacityObserved {
                    provider: claude_code::HARNESS.into(),
                    windows,
                });
            }
            Line::Result(result) => {
                let (terminal, outcome) = if result.is_error || result.subtype != "success" {
                    let reason = if result.errors.is_empty() {
                        format!("harness ended with {}", result.subtype)
                    } else {
                        format!("{}: {}", result.subtype, result.errors.join("; "))
                    };
                    (
                        Payload::RunFailed {
                            reason: reason.clone(),
                        },
                        RunOutcome::Failed(reason),
                    )
                } else {
                    let text = if result.text.is_empty() {
                        self.last_text.clone()
                    } else {
                        result.text.clone()
                    };
                    (
                        Payload::RunCompleted {
                            output: json!(text),
                        },
                        RunOutcome::Completed(text),
                    )
                };
                out.payloads.push(Payload::HarnessResult {
                    harness: claude_code::HARNESS.into(),
                    session_id: result.session_id,
                    subtype: result.subtype,
                    is_error: result.is_error,
                    cost_usd: result.cost_usd,
                    usage: result.usage,
                    permission_denials: result.permission_denials,
                });
                out.payloads.push(terminal);
                out.outcome = Some(outcome);
            }
            Line::Other(_) => {}
        }
        Ok(out)
    }
}

pub struct CodexDriver {
    model: String,
    cwd: String,
    version: String,
    session_id: String,
    last_text: String,
    /// Command items whose `item.started` already produced an intent.
    started: HashSet<String>,
}

impl CodexDriver {
    pub fn new(agent: &AgentSpec, cwd: &Path, version: String) -> Self {
        Self {
            model: if agent.model.is_empty() {
                "default".into()
            } else {
                agent.model.clone()
            },
            cwd: cwd.to_string_lossy().into_owned(),
            version,
            session_id: String::new(),
            last_text: String::new(),
            started: HashSet::new(),
        }
    }

    fn result(&self, subtype: &str, is_error: bool, usage: Value) -> Payload {
        Payload::HarnessResult {
            harness: codex::HARNESS.into(),
            session_id: self.session_id.clone(),
            subtype: subtype.into(),
            is_error,
            cost_usd: None,
            usage,
            permission_denials: json!([]),
        }
    }
}

impl HarnessDriver for CodexDriver {
    fn name(&self) -> &'static str {
        codex::HARNESS
    }

    fn binary(&self, spec: &HarnessSpec) -> PathBuf {
        codex::binary(spec)
    }

    fn command(
        &self,
        spec: &HarnessSpec,
        agent: &AgentSpec,
        cwd: &Path,
        prompt: &str,
        resume: Option<&str>,
        hooks: Option<(&HookEndpoint, &str)>,
    ) -> Command {
        codex::command(&self.binary(spec), spec, agent, cwd, prompt, resume, hooks)
    }

    fn interpret(&mut self, line: &str) -> Result<Interpretation, HarnessError> {
        use codex::{ItemKind, Line};
        let mut out = Interpretation::default();
        match codex::parse_line(line).map_err(|e| parse_error(e, line))? {
            Line::ThreadStarted { thread_id } => {
                self.session_id = thread_id.clone();
                out.session_started = Some(thread_id.clone());
                out.payloads.push(Payload::HarnessSessionStarted {
                    harness: codex::HARNESS.into(),
                    version: self.version.clone(),
                    session_id: thread_id,
                    cwd: self.cwd.clone(),
                    model: self.model.clone(),
                });
            }
            Line::ItemStarted(item) => {
                if let ItemKind::CommandExecution { command, .. } = item.kind {
                    self.started.insert(item.id.clone());
                    out.payloads.push(Payload::ToolIntent {
                        call_id: CallId(item.id),
                        tool: "Bash".into(),
                        class: ToolClass::SideEffecting,
                        input: json!({ "command": command }),
                    });
                }
            }
            Line::ItemCompleted(item) => match item.kind {
                ItemKind::AgentMessage { text } => {
                    if !text.trim().is_empty() {
                        self.last_text = text.trim().to_owned();
                    }
                    out.payloads.push(Payload::ModelResponseRecorded {
                        model: self.model.clone(),
                        response: json!({
                            "model": self.model,
                            "content": [{ "type": "text", "text": text }],
                        }),
                        context_seq: None,
                    });
                }
                ItemKind::CommandExecution {
                    command,
                    aggregated_output,
                    exit_code,
                    ..
                } => {
                    if !self.started.remove(&item.id) {
                        out.payloads.push(Payload::ToolIntent {
                            call_id: CallId(item.id.clone()),
                            tool: "Bash".into(),
                            class: ToolClass::SideEffecting,
                            input: json!({ "command": command }),
                        });
                    }
                    out.payloads.push(Payload::ToolResult {
                        call_id: CallId(item.id),
                        output: json!(aggregated_output),
                        is_error: exit_code != Some(0),
                    });
                }
                ItemKind::FileChange { changes } => {
                    out.payloads.push(Payload::ToolIntent {
                        call_id: CallId(item.id.clone()),
                        tool: "Edit".into(),
                        class: codex::file_change_class(&changes),
                        input: changes.clone(),
                    });
                    out.payloads.push(Payload::ToolResult {
                        call_id: CallId(item.id),
                        output: json!(codex::file_change_summary(&changes)),
                        is_error: false,
                    });
                }
                ItemKind::McpToolCall {
                    server,
                    tool,
                    arguments,
                    result,
                } => {
                    out.payloads.push(Payload::ToolIntent {
                        call_id: CallId(item.id.clone()),
                        tool: format!("mcp__{server}__{tool}"),
                        class: ToolClass::SideEffecting,
                        input: arguments,
                    });
                    out.payloads.push(Payload::ToolResult {
                        call_id: CallId(item.id),
                        output: result,
                        is_error: false,
                    });
                }
                ItemKind::WebSearch { query } => {
                    out.payloads.push(Payload::ToolIntent {
                        call_id: CallId(item.id.clone()),
                        tool: "WebSearch".into(),
                        class: ToolClass::Pure,
                        input: json!({ "query": query }),
                    });
                    out.payloads.push(Payload::ToolResult {
                        call_id: CallId(item.id),
                        output: json!(""),
                        is_error: false,
                    });
                }
                // Reasoning summaries, plans and Codex's own notices (for
                // example the hook-trust warning) are not steps of the run.
                ItemKind::Reasoning
                | ItemKind::TodoList
                | ItemKind::Error { .. }
                | ItemKind::Unknown(_) => {}
            },
            Line::TurnCompleted { usage } => {
                let text = self.last_text.clone();
                out.payloads.push(self.result("success", false, usage));
                out.payloads.push(Payload::RunCompleted {
                    output: json!(text),
                });
                out.outcome = Some(RunOutcome::Completed(text));
            }
            Line::TurnFailed { message } | Line::Error { message } => {
                let reason = format!("codex: {message}");
                out.payloads.push(self.result("error", true, Value::Null));
                out.payloads.push(Payload::RunFailed {
                    reason: reason.clone(),
                });
                out.outcome = Some(RunOutcome::Failed(reason));
            }
            Line::TurnStarted | Line::Other(_) => {}
        }
        Ok(out)
    }
}

/// A running harness: its process (leading its own process group) and a
/// line reader over its stdout. Dropping it kills the group.
pub struct HarnessProcess {
    child: Child,
    lines: Lines<BufReader<ChildStdout>>,
}

#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    #[error("starting harness: {0}")]
    Spawn(std::io::Error),
    #[error("reading harness output: {0}")]
    Read(std::io::Error),
    #[error("harness printed an unparsable line: {0}")]
    Parse(String),
    /// The process ended without a final result line, for example because
    /// it was killed or lost its network.
    #[error("harness exited early ({status}): {stderr}")]
    ExitedEarly { status: String, stderr: String },
}

impl HarnessProcess {
    pub fn spawn(mut cmd: tokio::process::Command) -> Result<Self, HarnessError> {
        // Its own process group, so an interrupt reaches the harness and
        // the tools it runs, as Ctrl-C in a terminal would.
        cmd.process_group(0);
        let mut child = cmd.spawn().map_err(HarnessError::Spawn)?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| HarnessError::Spawn(std::io::Error::other("stdout not piped")))?;
        Ok(Self {
            child,
            lines: BufReader::new(stdout).lines(),
        })
    }

    /// The next stdout line, or `None` at end of output.
    pub async fn next_line(&mut self) -> Result<Option<String>, HarnessError> {
        loop {
            match self.lines.next_line().await.map_err(HarnessError::Read)? {
                Some(line) if line.trim().is_empty() => continue,
                other => return Ok(other),
            }
        }
    }

    /// Waits for the process to end and returns its status and stderr.
    pub async fn finish(mut self) -> (Option<ExitStatus>, String) {
        let stderr = match self.child.stderr.take() {
            Some(mut err) => {
                let mut buf = String::new();
                tokio::io::AsyncReadExt::read_to_string(&mut err, &mut buf)
                    .await
                    .ok();
                buf
            }
            None => String::new(),
        };
        (self.child.wait().await.ok(), stderr.trim().to_owned())
    }

    /// Kills the harness and every tool it started.
    pub async fn kill(&mut self) {
        if let Some(pid) = self.child.id() {
            signal_group(pid, "KILL").await;
        }
        self.child.kill().await.ok();
    }

    /// Stops the process the way Ctrl-C would, so the harness can close
    /// its turn and save its session; kills the group if it is still
    /// running after `grace`. Its remaining output is discarded.
    pub async fn interrupt(&mut self, grace: Duration) {
        let group = self.child.id();
        if let Some(pid) = group {
            signal_group(pid, "INT").await;
        }
        let drain = async { while let Ok(Some(_)) = self.lines.next_line().await {} };
        if tokio::time::timeout(grace, drain).await.is_err() {
            if let Some(pid) = group {
                signal_group(pid, "KILL").await;
            }
            self.child.kill().await.ok();
        }
        self.child.wait().await.ok();
    }
}

impl Drop for HarnessProcess {
    /// A process still running when the worker lets go of it (the office
    /// is shutting down, the drive was dropped) takes its group with it.
    fn drop(&mut self) {
        if let Some(pid) = self.child.id() {
            let _ = std::process::Command::new("kill")
                .args(["-s", "KILL", "--", &format!("-{pid}")])
                .stderr(std::process::Stdio::null())
                .spawn();
        }
    }
}

/// Sends `signal` to a process group, without `unsafe`: through the
/// `kill` utility every Unix has.
async fn signal_group(pgid: u32, signal: &str) {
    let _ = tokio::process::Command::new("kill")
        .args(["-s", signal, "--", &format!("-{pgid}")])
        .stderr(std::process::Stdio::null())
        .status()
        .await;
}
