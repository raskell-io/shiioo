//! External agent harnesses driven as subprocesses (ADR-0009).

pub mod claude_code;

use std::process::ExitStatus;
use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::process::{Child, ChildStdout};

/// A running harness: its process and a line reader over its stdout.
/// Dropping it kills the process.
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

    pub async fn kill(&mut self) {
        self.child.kill().await.ok();
    }
}
