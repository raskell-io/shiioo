//! Where a harness process runs: the isolation levels of
//! `docs/design/tools-and-machines.md` §4.
//!
//! The worker builds the harness command as if it ran on the host, then
//! hands it to a [`Sandbox`], which either runs it as is (`worktree`),
//! translates it into `docker run …` with the worktree mounted
//! ([`container`]), or into `openshell sandbox exec …` ([`openshell`]).
//! The sandbox also decides how the harness's permission hook reaches the
//! office (the `shiioo` binary on the host; `curl` inside a container) and
//! what the run's `SandboxStarted` event says.

pub mod container;
pub mod egress;
pub mod openshell;

use crate::harness::claude_code::{HookCommand, HookEndpoint};
use crate::secrets::SecretSource;
use crate::spec::{HarnessKind, Isolation};
use serde_json::{Value, json};
use shiioo_core::{Payload, RunId};
use std::path::Path;
use tokio::process::Command;

/// What a sandbox needs to know about the run it isolates.
pub struct SandboxContext<'a> {
    /// The run whose sandbox state this is: the run itself, or for a
    /// follow-up the first run of its chain.
    pub run_id: RunId,
    pub harness: HarnessKind,
    /// The directory the harness works in (the run's worktree, or the
    /// machine's workspace).
    pub cwd: &'a Path,
    pub repo: Option<&'a Path>,
    /// Where per-run sandbox state lives (`.shiioo/sandboxes`).
    pub sandboxes_root: Option<&'a Path>,
    /// The office's hook endpoint as seen from the host.
    pub hooks: Option<&'a HookEndpoint>,
    /// Environment to inject, secrets already resolved.
    pub env: &'a [(String, String)],
    pub secrets: &'a dyn SecretSource,
    /// The harness binary the spec names. Without one, a sandbox runs the
    /// harness its image provides.
    pub binary_override: Option<&'a Path>,
}

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    /// The engine or CLI the level needs is missing or not running.
    #[error("{0}")]
    Unavailable(String),
    #[error("{program} {args}: {stderr}")]
    Command {
        program: String,
        args: String,
        stderr: String,
    },
    #[error("{0}")]
    Credentials(String),
    #[error("{0}")]
    Config(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

pub enum Sandbox {
    Worktree {
        env: Vec<(String, String)>,
        hooks: Option<HookEndpoint>,
        detail: Value,
    },
    Container(container::ContainerSandbox),
    Openshell(openshell::OpenshellSandbox),
}

impl Sandbox {
    /// Makes the level ready for this run: checks what it needs, builds
    /// images, starts sidecars. Nothing of the run itself executes yet.
    pub async fn prepare(
        isolation: &Isolation,
        ctx: &SandboxContext<'_>,
    ) -> Result<Self, SandboxError> {
        match isolation {
            Isolation::Worktree => Ok(Sandbox::Worktree {
                env: ctx.env.to_vec(),
                hooks: ctx.hooks.cloned(),
                detail: json!({
                    "cwd": ctx.cwd.to_string_lossy(),
                    "env": ctx.env.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
                }),
            }),
            Isolation::Container(spec) => container::ContainerSandbox::prepare(spec, ctx)
                .await
                .map(Sandbox::Container),
            Isolation::Openshell(spec) => openshell::OpenshellSandbox::prepare(spec, ctx)
                .await
                .map(Sandbox::Openshell),
        }
    }

    pub fn level(&self) -> &'static str {
        match self {
            Sandbox::Worktree { .. } => "worktree",
            Sandbox::Container(_) => "container",
            Sandbox::Openshell(_) => "openshell",
        }
    }

    /// The hook endpoint as the harness must use it from inside.
    pub fn hooks(&self) -> Option<HookEndpoint> {
        match self {
            Sandbox::Worktree { hooks, .. } => hooks.clone(),
            Sandbox::Container(c) => c.hooks.clone(),
            Sandbox::Openshell(o) => o.hooks.clone(),
        }
    }

    /// Turns the host command into the command that runs it at this level.
    pub fn wrap(&self, cmd: Command) -> Command {
        match self {
            Sandbox::Worktree { env, .. } => {
                let mut cmd = cmd;
                for (k, v) in env {
                    cmd.env(k, v);
                }
                cmd
            }
            Sandbox::Container(c) => c.wrap(cmd),
            Sandbox::Openshell(o) => o.wrap(cmd),
        }
    }

    /// The event recorded before the harness starts.
    pub fn started(&self) -> Payload {
        Payload::SandboxStarted {
            level: self.level().into(),
            detail: match self {
                Sandbox::Worktree { detail, .. } => detail.clone(),
                Sandbox::Container(c) => c.detail(),
                Sandbox::Openshell(o) => o.detail(),
            },
        }
    }

    /// Stops and removes whatever the level started for this run. The
    /// run's worktree and sandbox home stay for a resume.
    pub async fn teardown(&self) {
        match self {
            Sandbox::Worktree { .. } => {}
            Sandbox::Container(c) => c.teardown().await,
            Sandbox::Openshell(o) => o.teardown().await,
        }
    }
}

/// The office's hook endpoint rewritten for a process inside a sandbox:
/// the host is reached as `host.docker.internal`, and the hook is a `curl`
/// call because the `shiioo` binary is not inside.
pub(crate) fn hooks_from_inside(hooks: Option<&HookEndpoint>) -> Option<HookEndpoint> {
    hooks.map(|h| HookEndpoint {
        server_url: with_host(&h.server_url, "host.docker.internal"),
        token: h.token.clone(),
        command: HookCommand::Curl,
    })
}

/// `http://127.0.0.1:7878` → `http://<host>:7878`.
pub(crate) fn with_host(url: &str, host: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    let (authority, path) = rest.split_once('/').map_or((rest, ""), |(a, p)| (a, p));
    let port = authority
        .rsplit_once(':')
        .filter(|(h, p)| !h.ends_with(']') || p.chars().all(|c| c.is_ascii_digit()))
        .map(|(_, p)| p)
        .filter(|p| p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty());
    let mut out = format!("{scheme}://{host}");
    if let Some(port) = port {
        out.push(':');
        out.push_str(port);
    }
    if !path.is_empty() {
        out.push('/');
        out.push_str(path);
    }
    out
}

/// The port of an `http://host:port` URL, if it names one.
pub(crate) fn port_of(url: &str) -> Option<u16> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split('/').next()?;
    authority.rsplit_once(':')?.1.parse().ok()
}

/// Runs a command to completion and returns its stdout, or the stderr as
/// an error.
pub(crate) async fn output(program: &str, args: &[&str]) -> Result<String, SandboxError> {
    let out = Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| {
            SandboxError::Unavailable(format!("{program} is not available ({e})"))
        })?;
    if !out.status.success() {
        return Err(SandboxError::Command {
            program: program.into(),
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// First 12 hex characters of the SHA-256 of `text`: an image tag that
/// changes when the Dockerfile does.
pub(crate) fn short_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(text.as_bytes());
    hex::encode(&digest[..6])
}

/// The first eight characters of a run id, as in branch and container names.
pub(crate) fn short_run(run_id: RunId) -> String {
    run_id.to_string()[..8].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_the_host_and_keeps_the_port() {
        assert_eq!(
            with_host("http://127.0.0.1:7878", "host.docker.internal"),
            "http://host.docker.internal:7878"
        );
        assert_eq!(
            with_host("http://0.0.0.0:80/v1", "h"),
            "http://h:80/v1"
        );
        assert_eq!(with_host("http://localhost", "h"), "http://h");
        assert_eq!(port_of("http://127.0.0.1:7878"), Some(7878));
        assert_eq!(port_of("http://localhost/x"), None);
    }

    #[test]
    fn hooks_inside_use_curl() {
        let host = HookEndpoint {
            server_url: "http://127.0.0.1:7878".into(),
            token: "t".into(),
            command: HookCommand::Exe("/usr/local/bin/shiioo".into()),
        };
        let inside = hooks_from_inside(Some(&host)).unwrap_or_else(|| panic!("no hooks"));
        assert_eq!(inside.server_url, "http://host.docker.internal:7878");
        assert_eq!(inside.command, HookCommand::Curl);
        assert!(hooks_from_inside(None).is_none());
    }

    #[test]
    fn short_hash_is_stable() {
        assert_eq!(short_hash("x").len(), 12);
        assert_eq!(short_hash("x"), short_hash("x"));
        assert_ne!(short_hash("x"), short_hash("y"));
    }
}
