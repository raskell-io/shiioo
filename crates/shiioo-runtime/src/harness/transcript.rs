//! What happened during a takeover, read back from the harness's own
//! session file: the owner's messages, the agent's replies and its tool
//! calls, as the events a headless run would have recorded.
//!
//! The terminal stream itself is not the record (it is screen drawing);
//! the harnesses write each conversation to a JSONL file, and that is what
//! is imported. A snapshot taken before the takeover remembers which lines
//! were already there; the import takes the lines added since, from the
//! session file or from a new one the harness started for the same
//! working directory (a resume may fork the session).

use super::{ClaudeCodeDriver, HarnessDriver};
use crate::spec::HarnessKind;
use serde_json::{Value, json};
use shiioo_core::{CallId, Payload, ToolClass};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Where the harnesses keep their sessions.
#[derive(Clone, Debug)]
pub struct Homes {
    /// `CLAUDE_CONFIG_DIR`, else `~/.claude`; sessions under `projects/`.
    pub claude: PathBuf,
    /// `CODEX_HOME`, else `~/.codex`; sessions under `sessions/`.
    pub codex: PathBuf,
}

impl Homes {
    pub fn from_env() -> Self {
        let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
        Self {
            claude: std::env::var_os("CLAUDE_CONFIG_DIR")
                .map_or_else(|| home.join(".claude"), PathBuf::from),
            codex: std::env::var_os("CODEX_HOME")
                .map_or_else(|| home.join(".codex"), PathBuf::from),
        }
    }
}

/// The session files as they were before the takeover.
pub struct Snapshot {
    kind: HarnessKind,
    homes: Homes,
    cwd: PathBuf,
    session: Option<String>,
    started: SystemTime,
    seen: HashSet<String>,
}

/// What the import found: events to append, and the session the next
/// headless process should resume.
#[derive(Debug, Default)]
pub struct Imported {
    pub payloads: Vec<Payload>,
    pub session: Option<String>,
}

impl Snapshot {
    pub fn take(kind: HarnessKind, homes: Homes, cwd: &Path, session: Option<&str>) -> Self {
        let mut snapshot = Self {
            kind,
            homes,
            cwd: cwd.to_path_buf(),
            session: session.map(str::to_owned),
            started: SystemTime::now(),
            seen: HashSet::new(),
        };
        for file in snapshot.files(None) {
            for line in read_lines(&file) {
                snapshot.seen.insert(snapshot.key(&line));
            }
        }
        snapshot
    }

    /// The lines added since the snapshot, as events.
    pub fn import(&self) -> Imported {
        let mut files = self.files(Some(self.started));
        files.sort_by_key(|f| modified(f));
        let mut out = Imported::default();
        let mut claude = ClaudeCodeDriver::default();
        let mut polls = HashSet::new();
        for file in &files {
            for line in read_lines(file) {
                if self.seen.contains(&self.key(&line)) {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                match self.kind {
                    HarnessKind::ClaudeCode => {
                        if let Some(id) = v["sessionId"].as_str() {
                            out.session = Some(id.to_owned());
                        }
                        out.payloads.extend(claude_line(&mut claude, &v, &line));
                    }
                    HarnessKind::Codex => {
                        if v["type"] == "session_meta"
                            && let Some(id) = v["payload"]["id"].as_str()
                        {
                            out.session = Some(id.to_owned());
                        }
                        out.payloads.extend(codex_line(&v, &mut polls));
                    }
                }
            }
        }
        if out.session.is_none() && !files.is_empty() {
            out.session.clone_from(&self.session);
        }
        out
    }

    /// The session files that matter: the session's own, and (with
    /// `since`) any changed since then for the same working directory.
    fn files(&self, since: Option<SystemTime>) -> Vec<PathBuf> {
        let fresh = |f: &Path| since.is_none_or(|t| modified(f) >= Some(t));
        match self.kind {
            HarnessKind::ClaudeCode => {
                let dir = self
                    .homes
                    .claude
                    .join("projects")
                    .join(project_dir(&self.cwd));
                let mut files: Vec<PathBuf> = list(&dir)
                    .into_iter()
                    .filter(|f| f.extension().is_some_and(|e| e == "jsonl"))
                    .collect();
                if since.is_none() {
                    // Before: only the session being resumed.
                    files.retain(|f| {
                        self.session
                            .as_deref()
                            .is_some_and(|s| f.file_stem().is_some_and(|n| n == s))
                    });
                }
                files.retain(|f| fresh(f));
                files
            }
            HarnessKind::Codex => {
                let mut files = Vec::new();
                walk(&self.homes.codex.join("sessions"), &mut files);
                files.retain(|f| {
                    let name = f.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                    if !name.starts_with("rollout-") || !name.ends_with(".jsonl") {
                        return false;
                    }
                    let own = self.session.as_deref().is_some_and(|s| name.contains(s));
                    match since {
                        None => own,
                        Some(_) => fresh(f) && (own || codex_cwd(f).as_deref() == Some(&self.cwd)),
                    }
                });
                files
            }
        }
    }

    /// What identifies a line across reads: Claude Code's `uuid`, else the
    /// line itself.
    fn key(&self, line: &str) -> String {
        if self.kind == HarnessKind::ClaudeCode
            && let Ok(v) = serde_json::from_str::<Value>(line)
            && let Some(id) = v["uuid"].as_str()
        {
            return id.to_owned();
        }
        line.to_owned()
    }
}

/// Claude Code's directory for a working directory: every character but
/// letters and digits becomes `-`.
pub fn project_dir(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn claude_line(driver: &mut ClaudeCodeDriver, v: &Value, line: &str) -> Vec<Payload> {
    if v["isSidechain"] == true || v["isMeta"] == true {
        return Vec::new();
    }
    match v["type"].as_str() {
        Some("assistant") => driver
            .interpret(line)
            .map(|i| i.payloads)
            .unwrap_or_default(),
        Some("user") => {
            let content = &v["message"]["content"];
            let has_results = content
                .as_array()
                .is_some_and(|a| a.iter().any(|b| b["type"] == "tool_result"));
            if has_results {
                return driver
                    .interpret(line)
                    .map(|i| i.payloads)
                    .unwrap_or_default();
            }
            let text = match content {
                Value::String(s) => s.clone(),
                Value::Array(blocks) => blocks
                    .iter()
                    .filter(|b| b["type"] == "text")
                    .filter_map(|b| b["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => String::new(),
            };
            owner_text(&text)
                .map(|content| {
                    vec![Payload::UserMessage {
                        author: "owner".into(),
                        content,
                    }]
                })
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

/// What the owner typed, as it should read in the log; `None` for the
/// harness's own bookkeeping (slash-command echoes, caveats).
fn owner_text(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    // Claude Code's `!` shell escape: the command and what it printed.
    if let Some(cmd) = between(text, "<bash-input>", "</bash-input>") {
        return Some(format!("! {cmd}"));
    }
    if text.contains("<bash-stdout>") || text.contains("<bash-stderr>") {
        let out = [
            between(text, "<bash-stdout>", "</bash-stdout>"),
            between(text, "<bash-stderr>", "</bash-stderr>"),
        ]
        .into_iter()
        .flatten()
        .filter(|s| !s.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
        return (!out.is_empty()).then(|| format!("(shell output)\n{out}"));
    }
    if text.starts_with('<') || text.starts_with("Caveat:") {
        return None;
    }
    Some(text.to_owned())
}

fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(&text[start..end])
}

/// Codex's tools for checking on a command still running (`wait`,
/// `write_stdin`): part of the command's call, not calls of their own.
const CODEX_POLLS: &[&str] = &["wait", "write_stdin"];

fn codex_line(v: &Value, polls: &mut HashSet<String>) -> Vec<Payload> {
    if v["type"] != "response_item" {
        return Vec::new();
    }
    let p = &v["payload"];
    let text_of = |content: &Value| -> String {
        content
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    };
    match p["type"].as_str() {
        Some("message") => {
            let text = text_of(&p["content"]);
            match p["role"].as_str() {
                Some("assistant") if !text.trim().is_empty() => {
                    vec![Payload::ModelResponseRecorded {
                        model: "codex".into(),
                        response: json!({
                            "role": "assistant",
                            "content": [{ "type": "text", "text": text }],
                        }),
                        context_seq: None,
                    }]
                }
                Some("user") => owner_text(&text)
                    .filter(|t| !t.starts_with("# AGENTS.md"))
                    .map(|content| {
                        vec![Payload::UserMessage {
                            author: "owner".into(),
                            content,
                        }]
                    })
                    .unwrap_or_default(),
                _ => Vec::new(),
            }
        }
        Some("function_call" | "custom_tool_call" | "local_shell_call") => {
            let call_id = p["call_id"].as_str().unwrap_or_default().to_owned();
            if p["name"].as_str().is_some_and(|n| CODEX_POLLS.contains(&n)) {
                polls.insert(call_id);
                return Vec::new();
            }
            let (tool, input) = codex_call(p);
            vec![Payload::ToolIntent {
                call_id: CallId(call_id),
                class: ToolClass::SideEffecting,
                tool,
                input,
            }]
        }
        Some("function_call_output" | "custom_tool_call_output") => {
            if p["call_id"].as_str().is_some_and(|id| polls.contains(id)) {
                return Vec::new();
            }
            let output = match &p["output"] {
                Value::String(s) => serde_json::from_str::<Value>(s)
                    .ok()
                    .and_then(|o| o["output"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| s.clone()),
                other => text_of(other),
            };
            vec![Payload::ToolResult {
                call_id: CallId(p["call_id"].as_str().unwrap_or_default().to_owned()),
                output: json!(output),
                is_error: false,
            }]
        }
        _ => Vec::new(),
    }
}

/// A Codex tool call as the log shows tool calls: shell commands as
/// `Bash { command }`, the rest by name with their input.
fn codex_call(p: &Value) -> (String, Value) {
    let name = p["name"].as_str().unwrap_or("tool");
    let command = match p["type"].as_str() {
        Some("local_shell_call") => joined(&p["action"]["command"]),
        Some("function_call") => {
            serde_json::from_str::<Value>(p["arguments"].as_str().unwrap_or("{}"))
                .ok()
                .and_then(|a| {
                    joined(&a["command"]).or_else(|| a["cmd"].as_str().map(str::to_owned))
                })
        }
        // `exec` wraps the command in a small script: exec_command({cmd:"…"}).
        _ => p["input"].as_str().and_then(script_cmd),
    };
    match command {
        Some(command) => ("Bash".into(), json!({ "command": command })),
        None => (
            name.to_owned(),
            p.get("input")
                .cloned()
                .or_else(|| p.get("arguments").cloned())
                .unwrap_or(Value::Null),
        ),
    }
}

fn joined(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => {
            let parts: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
            // `["bash", "-lc", "cargo test"]` reads as the command.
            match parts.as_slice() {
                [_, flag, cmd] if flag.starts_with('-') && flag.ends_with('c') => {
                    Some((*cmd).to_owned())
                }
                _ => Some(parts.join(" ")),
            }
        }
        _ => None,
    }
}

/// `cmd:"python3 wait.py"` (or `"cmd":"…"`) out of an `exec` script.
fn script_cmd(script: &str) -> Option<String> {
    let at = script.find("cmd")?;
    let rest = script[at + 3..].trim_start_matches('"').trim_start();
    let rest = rest.strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => return None,
            },
            c => out.push(c),
        }
    }
    None
}

fn codex_cwd(file: &Path) -> Option<PathBuf> {
    let first = read_lines(file).into_iter().next()?;
    let v: Value = serde_json::from_str(&first).ok()?;
    (v["type"] == "session_meta")
        .then(|| PathBuf::from(v["payload"]["cwd"].as_str().unwrap_or_default()))
}

fn read_lines(file: &Path) -> Vec<String> {
    std::fs::read_to_string(file)
        .map(|s| {
            s.lines()
                .filter(|l| !l.trim().is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn list(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default()
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for path in list(dir) {
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn modified(file: &Path) -> Option<SystemTime> {
    std::fs::metadata(file).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, lines: &[Value]) {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{e}"));
        }
        let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(path, body).unwrap_or_else(|e| panic!("{e}"));
    }

    fn homes(dir: &Path) -> Homes {
        Homes {
            claude: dir.join("claude"),
            codex: dir.join("codex"),
        }
    }

    #[test]
    fn project_dirs_replace_everything_but_letters_and_digits() {
        assert_eq!(
            project_dir(Path::new("/private/tmp/a.b/x_y")),
            "-private-tmp-a-b-x-y"
        );
    }

    #[test]
    fn claude_code_lines_added_during_the_takeover_become_events() {
        let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let cwd = Path::new("/work/tree");
        let file = tmp
            .path()
            .join("claude/projects")
            .join(project_dir(cwd))
            .join("s1.jsonl");
        let before = json!({"type": "user", "uuid": "u0", "sessionId": "s1",
            "message": {"role": "user", "content": "Fix the test."}});
        write(&file, std::slice::from_ref(&before));
        let snap = Snapshot::take(HarnessKind::ClaudeCode, homes(tmp.path()), cwd, Some("s1"));
        write(
            &file,
            &[
                before,
                json!({"type": "user", "uuid": "u1", "sessionId": "s1",
                "message": {"role": "user", "content": "Use the backoff helper."}}),
                json!({"type": "assistant", "uuid": "a1", "sessionId": "s1",
                "message": {"model": "m", "role": "assistant", "content": [
                    {"type": "text", "text": "Running it."},
                    {"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "cargo test"}}]}}),
                json!({"type": "user", "uuid": "u2", "sessionId": "s1",
                "message": {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": "ok", "is_error": false}]}}),
                json!({"type": "user", "uuid": "u3", "sessionId": "s1", "isMeta": true,
                "message": {"role": "user", "content": "Caveat: the messages below…"}}),
                json!({"type": "user", "uuid": "u4", "sessionId": "s1",
                "message": {"role": "user", "content": "<bash-input>git status</bash-input>"}}),
            ],
        );
        let imported = snap.import();
        assert_eq!(imported.session.as_deref(), Some("s1"));
        let kinds: Vec<&str> = imported
            .payloads
            .iter()
            .map(|p| match p {
                Payload::UserMessage { .. } => "message",
                Payload::ModelResponseRecorded { .. } => "response",
                Payload::ToolIntent { .. } => "intent",
                Payload::ToolResult { .. } => "result",
                _ => "other",
            })
            .collect();
        assert_eq!(
            kinds,
            ["message", "response", "intent", "result", "message"]
        );
        assert!(
            matches!(&imported.payloads[0], Payload::UserMessage { content, .. } if content == "Use the backoff helper.")
        );
        assert!(
            matches!(&imported.payloads[4], Payload::UserMessage { content, .. } if content == "! git status")
        );
    }

    #[test]
    fn a_forked_claude_session_is_followed_without_repeating_history() {
        let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let cwd = Path::new("/work/tree");
        let dir = tmp.path().join("claude/projects").join(project_dir(cwd));
        let old = json!({"type": "user", "uuid": "u0", "sessionId": "s1",
            "message": {"role": "user", "content": "Fix the test."}});
        write(&dir.join("s1.jsonl"), std::slice::from_ref(&old));
        let snap = Snapshot::take(HarnessKind::ClaudeCode, homes(tmp.path()), cwd, Some("s1"));
        // The resume copied the history into a new session file.
        write(
            &dir.join("s2.jsonl"),
            &[
                json!({"type": "user", "uuid": "u0", "sessionId": "s2",
                "message": {"role": "user", "content": "Fix the test."}}),
                json!({"type": "user", "uuid": "u9", "sessionId": "s2",
                "message": {"role": "user", "content": "Now the README."}}),
            ],
        );
        let imported = snap.import();
        assert_eq!(imported.session.as_deref(), Some("s2"));
        assert_eq!(imported.payloads.len(), 1);
    }

    #[test]
    fn codex_rollout_items_become_events() {
        let tmp = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let cwd = Path::new("/work/tree");
        let file = tmp
            .path()
            .join("codex/sessions/2026/10/31/rollout-2026-10-31T12-00-00-th1.jsonl");
        let meta = json!({"type": "session_meta", "payload": {"id": "th1", "cwd": "/work/tree"}});
        write(&file, std::slice::from_ref(&meta));
        let snap = Snapshot::take(HarnessKind::Codex, homes(tmp.path()), cwd, Some("th1"));
        write(
            &file,
            &[
                meta,
                json!({"type": "response_item", "payload": {"type": "message", "role": "user",
                "content": [{"type": "input_text", "text": "Stop, use the helper."}]}}),
                json!({"type": "response_item", "payload": {"type": "message", "role": "user",
                "content": [{"type": "input_text", "text": "<environment_context>…</environment_context>"}]}}),
                json!({"type": "response_item", "payload": {"type": "custom_tool_call", "call_id": "c1", "name": "exec",
                "input": "text(await tools.exec_command({cmd:\"cargo test -p net\",\"yield_time_ms\":1000}));"}}),
                json!({"type": "response_item", "payload": {"type": "custom_tool_call_output", "call_id": "c1",
                "output": [{"type": "input_text", "text": "test result: ok"}]}}),
                json!({"type": "response_item", "payload": {"type": "custom_tool_call", "call_id": "c2", "name": "wait",
                "input": "{\"cell_id\":\"1\"}"}}),
                json!({"type": "response_item", "payload": {"type": "custom_tool_call_output", "call_id": "c2",
                "output": [{"type": "input_text", "text": "still running"}]}}),
                json!({"type": "response_item", "payload": {"type": "message", "role": "assistant",
                "content": [{"type": "output_text", "text": "Done."}]}}),
            ],
        );
        let imported = snap.import();
        assert_eq!(imported.session.as_deref(), Some("th1"));
        assert_eq!(imported.payloads.len(), 4, "{:#?}", imported.payloads);
        assert!(
            matches!(&imported.payloads[1], Payload::ToolIntent { tool, input, .. }
            if tool == "Bash" && input["command"] == "cargo test -p net")
        );
        assert!(
            matches!(&imported.payloads[2], Payload::ToolResult { output, .. } if output == "test result: ok")
        );
    }

    #[test]
    fn shell_commands_come_out_of_codex_tool_calls() {
        assert_eq!(
            script_cmd(r#"exec_command({cmd:"ls -la","x":1})"#).as_deref(),
            Some("ls -la")
        );
        assert_eq!(
            script_cmd(r#"exec_command({"cmd": "echo \"hi\""})"#).as_deref(),
            Some("echo \"hi\"")
        );
        assert_eq!(
            joined(&json!(["bash", "-lc", "cargo test"])).as_deref(),
            Some("cargo test")
        );
    }
}
