use crate::model::{BoxFuture, ToolSpec};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use shiioo_core::ToolClass;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Recorded tool output is capped; the head and tail of longer output are
/// kept (`docs/design/tools-and-machines.md`).
const OUTPUT_KEEP: usize = 32 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
        }
    }
}

pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;

    /// How this call behaves if it may have run but its result was never
    /// recorded. Decided per call: the same tool can be pure for one input
    /// and side-effecting for another.
    fn class(&self, input: &Value) -> ToolClass;

    fn execute<'a>(&'a self, input: &'a Value, workspace: &'a Path) -> BoxFuture<'a, ToolOutput>;
}

/// The tools available on a machine, by name.
#[derive(Clone, Default)]
pub struct Toolbox {
    tools: BTreeMap<String, Arc<dyn Tool>>,
}

impl Toolbox {
    /// `read_file`, `write_file` and `shell`.
    pub fn builtin() -> Self {
        Self::default()
            .with(Arc::new(ReadFile))
            .with(Arc::new(WriteFile))
            .with(Arc::new(Shell {
                timeout: Duration::from_secs(120),
            }))
    }

    pub fn with(mut self, tool: Arc<dyn Tool>) -> Self {
        self.tools.insert(tool.spec().name, tool);
        self
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    /// Specs for the named tools that exist here, in the order given.
    pub fn specs(&self, names: &[String]) -> Vec<ToolSpec> {
        names
            .iter()
            .filter_map(|n| self.tools.get(n))
            .map(|t| t.spec())
            .collect()
    }
}

/// Resolves a model-supplied path inside the workspace, refusing absolute
/// paths and any `..` component.
fn resolve(workspace: &Path, path: &str) -> Result<PathBuf, ToolOutput> {
    let rel = Path::new(path);
    if rel.is_absolute()
        || rel
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(ToolOutput::error(format!(
            "path must be relative to the workspace and must not contain '..': {path}"
        )));
    }
    Ok(workspace.join(rel))
}

/// Length-prefixed SHA-256 over `parts`, so ("ab", "c") and ("a", "bc")
/// get different keys.
pub(crate) fn digest_hex(parts: &[&[u8]]) -> String {
    let mut h = Sha256::new();
    for part in parts {
        h.update((part.len() as u64).to_be_bytes());
        h.update(part);
    }
    hex::encode(h.finalize())
}

fn str_arg<'a>(input: &'a Value, key: &str) -> Result<&'a str, ToolOutput> {
    input[key]
        .as_str()
        .ok_or_else(|| ToolOutput::error(format!("missing string argument '{key}'")))
}

fn truncate(s: String) -> String {
    if s.len() <= 2 * OUTPUT_KEEP {
        return s;
    }
    let mut head_end = OUTPUT_KEEP;
    while !s.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = s.len() - OUTPUT_KEEP;
    while !s.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    format!(
        "{}\n… [{} bytes omitted] …\n{}",
        &s[..head_end],
        tail_start - head_end,
        &s[tail_start..]
    )
}

pub struct ReadFile;

impl Tool for ReadFile {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_file".into(),
            description: "Read a UTF-8 text file from the workspace.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "path": { "type": "string", "description": "Path relative to the workspace" } },
                "required": ["path"],
                "additionalProperties": false
            }),
        }
    }

    fn class(&self, _: &Value) -> ToolClass {
        ToolClass::Pure
    }

    fn execute<'a>(&'a self, input: &'a Value, workspace: &'a Path) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let path = match str_arg(input, "path").and_then(|p| resolve(workspace, p)) {
                Ok(p) => p,
                Err(e) => return e,
            };
            match tokio::fs::read_to_string(&path).await {
                Ok(s) => ToolOutput::ok(truncate(s)),
                Err(e) => ToolOutput::error(format!("reading {}: {e}", path.display())),
            }
        })
    }
}

pub struct WriteFile;

impl Tool for WriteFile {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "write_file".into(),
            description: "Create or overwrite a text file in the workspace.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path relative to the workspace" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
        }
    }

    /// Writing the same content to the same path twice has the same effect
    /// as writing it once.
    fn class(&self, input: &Value) -> ToolClass {
        ToolClass::Idempotent {
            key: digest_hex(&[
                input["path"].as_str().unwrap_or_default().as_bytes(),
                input["content"].as_str().unwrap_or_default().as_bytes(),
            ]),
        }
    }

    fn execute<'a>(&'a self, input: &'a Value, workspace: &'a Path) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let (path, content) = match str_arg(input, "path")
                .and_then(|p| resolve(workspace, p))
                .and_then(|p| Ok((p, str_arg(input, "content")?)))
            {
                Ok(v) => v,
                Err(e) => return e,
            };
            if let Some(parent) = path.parent()
                && let Err(e) = tokio::fs::create_dir_all(parent).await
            {
                return ToolOutput::error(format!("creating {}: {e}", parent.display()));
            }
            match tokio::fs::write(&path, content).await {
                Ok(()) => ToolOutput::ok(format!(
                    "wrote {} bytes to {}",
                    content.len(),
                    path.display()
                )),
                Err(e) => ToolOutput::error(format!("writing {}: {e}", path.display())),
            }
        })
    }
}

pub struct Shell {
    pub timeout: Duration,
}

impl Tool for Shell {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "shell".into(),
            description:
                "Run a shell command (sh -c) in the workspace and return its exit code and output."
                    .into(),
            input_schema: json!({
                "type": "object",
                "properties": { "command": { "type": "string" } },
                "required": ["command"],
                "additionalProperties": false
            }),
        }
    }

    fn class(&self, _: &Value) -> ToolClass {
        ToolClass::SideEffecting
    }

    fn execute<'a>(&'a self, input: &'a Value, workspace: &'a Path) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let command = match str_arg(input, "command") {
                Ok(c) => c,
                Err(e) => return e,
            };
            let child = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(command)
                .current_dir(workspace)
                .kill_on_drop(true)
                .output();
            match tokio::time::timeout(self.timeout, child).await {
                Err(_) => ToolOutput::error(format!("timed out after {:?}", self.timeout)),
                Ok(Err(e)) => ToolOutput::error(format!("starting command: {e}")),
                Ok(Ok(out)) => {
                    let text = format!(
                        "exit code: {}\nstdout:\n{}\nstderr:\n{}",
                        out.status.code().map_or("signal".into(), |c| c.to_string()),
                        String::from_utf8_lossy(&out.stdout),
                        String::from_utf8_lossy(&out.stderr),
                    );
                    ToolOutput {
                        content: truncate(text),
                        is_error: !out.status.success(),
                    }
                }
            }
        })
    }
}
