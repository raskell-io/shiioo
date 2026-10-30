//! The `openshell` level: one NVIDIA OpenShell sandbox per run, driven
//! through the `openshell` CLI against the gateway the owner selected.
//!
//! Experimental. Written against the OpenShell documentation (sandbox
//! lifecycle, policy schema v1) and exercised here only at the level of
//! the commands it builds; it has not been run against a live gateway.
//! What it does:
//!
//! - writes a policy with a `network_policies` section generated from the
//!   run's egress rules (plus the office's API host, for the hook) and
//!   `enforcement: enforce` on every endpoint;
//! - `openshell sandbox create --name shiioo-<run> --policy … --upload
//!   <worktree>:/workspace --detach -- sleep infinity`, uploading Codex's
//!   auth file alongside when the harness is Codex;
//! - runs the harness with `openshell sandbox exec -n … --workdir
//!   /workspace -- <harness> …`, streaming its output like any other
//!   harness process;
//! - `openshell sandbox delete` afterwards.
//!
//! The upload is a copy, not a mount: the agent's work does not land in
//! the local worktree. It reaches the owner through git, so a task at this
//! level should end with a push, and the egress rules must allow the
//! remote. The run's branch name is in the task text for that reason.

use super::{SandboxContext, SandboxError, hooks_from_inside, output, port_of, short_run};
use crate::harness::claude_code::HookEndpoint;
use crate::secrets::CLAUDE_OAUTH_TOKEN;
use crate::spec::{Egress, HarnessKind, OpenshellSpec};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

pub const WORKDIR: &str = "/workspace";

pub struct OpenshellSandbox {
    bin: PathBuf,
    name: String,
    image: String,
    policy: PathBuf,
    env: Vec<(String, String)>,
    program: Option<String>,
    egress: Egress,
    pub(crate) hooks: Option<HookEndpoint>,
    created: bool,
}

impl OpenshellSandbox {
    pub async fn prepare(
        spec: &OpenshellSpec,
        ctx: &SandboxContext<'_>,
    ) -> Result<Self, SandboxError> {
        let bin = binary();
        let root = ctx.sandboxes_root.ok_or_else(|| {
            SandboxError::Config("this machine has no sandbox directory for openshell runs".into())
        })?;
        let bin_str = bin.to_string_lossy().into_owned();
        output(&bin_str, &["--version"]).await.map_err(|e| {
            SandboxError::Unavailable(format!(
                "the openshell CLI is not available ({e}); install it and select a gateway, \
                 see https://docs.nvidia.com/openshell/"
            ))
        })?;
        let dir = root.join(ctx.run_id.to_string());
        std::fs::create_dir_all(&dir)?;
        let office_port = ctx.hooks.and_then(|h| port_of(&h.server_url));
        let policy = dir.join("openshell-policy.yaml");
        std::fs::write(&policy, policy_yaml(&spec.egress, office_port))?;

        let mut env = ctx.env.to_vec();
        let mut uploads = vec![format!("{}:{WORKDIR}", ctx.cwd.to_string_lossy())];
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        match ctx.harness {
            HarnessKind::Codex => {
                let auth = home.join(".codex").join("auth.json");
                if !auth.is_file() {
                    return Err(SandboxError::Credentials(format!(
                        "codex is not logged in on this machine ({} missing); run `codex login`",
                        auth.display()
                    )));
                }
                uploads.push(format!("{}:/root/.codex/auth.json", auth.display()));
            }
            HarnessKind::ClaudeCode => {
                if !env.iter().any(|(k, _)| k == "CLAUDE_CODE_OAUTH_TOKEN") {
                    let token = ctx
                        .secrets
                        .get(CLAUDE_OAUTH_TOKEN)
                        .or_else(|| std::env::var("CLAUDE_CODE_OAUTH_TOKEN").ok())
                        .filter(|t| !t.is_empty())
                        .ok_or_else(|| {
                            SandboxError::Credentials(format!(
                                "no Claude Code login a sandbox can use: run `claude setup-token` \
                                 and `shiioo secret set {CLAUDE_OAUTH_TOKEN}`"
                            ))
                        })?;
                    env.push(("CLAUDE_CODE_OAUTH_TOKEN".into(), token));
                }
            }
        }

        let name = format!("shiioo-{}", short_run(ctx.run_id));
        output(&bin_str, &["sandbox", "delete", &name]).await.ok();
        let mut args: Vec<String> = create_args(&name, spec, &policy, &uploads);
        args.extend(["--".into(), "sleep".into(), "infinity".into()]);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        output(&bin_str, &arg_refs).await?;
        Ok(Self {
            bin,
            name,
            image: spec.image.clone(),
            policy,
            env,
            program: ctx.binary_override.is_none().then(|| {
                match ctx.harness {
                    HarnessKind::ClaudeCode => "claude",
                    HarnessKind::Codex => "codex",
                }
                .to_owned()
            }),
            egress: spec.egress.clone(),
            hooks: hooks_from_inside(ctx.hooks),
            created: true,
        })
    }

    /// `openshell sandbox exec -n <name> --workdir /workspace --env … --
    /// <harness> <args>`.
    pub fn wrap(&self, cmd: Command) -> Command {
        let std_cmd = cmd.as_std();
        let program = self
            .program
            .clone()
            .unwrap_or_else(|| std_cmd.get_program().to_string_lossy().into_owned());
        let mut exec = Command::new(&self.bin);
        exec.args(["sandbox", "exec", "-n", &self.name, "--workdir", WORKDIR]);
        for (k, v) in std_cmd.get_envs().filter_map(|(k, v)| Some((k, v?))) {
            exec.arg("--env")
                .arg(format!("{}={}", k.to_string_lossy(), v.to_string_lossy()));
        }
        for (k, v) in &self.env {
            exec.arg("--env").arg(format!("{k}={v}"));
        }
        exec.arg("--").arg(program).args(std_cmd.get_args());
        exec.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        exec
    }

    pub fn detail(&self) -> Value {
        json!({
            "sandbox": self.name,
            "image": if self.image.is_empty() { Value::Null } else { Value::from(self.image.as_str()) },
            "policy": self.policy.to_string_lossy(),
            "workdir": WORKDIR,
            "egress": match &self.egress {
                Egress::Allowlist { hosts } => json!({ "mode": "allowlist", "hosts": hosts }),
                Egress::Open => json!({ "mode": "open" }),
            },
            "env": self.env.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            "note": "work is uploaded as a copy; results come back through git",
        })
    }

    pub async fn teardown(&self) {
        if self.created {
            output(
                &self.bin.to_string_lossy(),
                &["sandbox", "delete", &self.name],
            )
            .await
            .ok();
        }
    }
}

/// `SHIIOO_OPENSHELL_BIN`, else `openshell` on the path.
pub fn binary() -> PathBuf {
    std::env::var_os("SHIIOO_OPENSHELL_BIN")
        .map_or_else(|| PathBuf::from("openshell"), PathBuf::from)
}

/// The arguments of `openshell sandbox create` for a run, without the
/// trailing `-- <command>`.
pub fn create_args(
    name: &str,
    spec: &OpenshellSpec,
    policy: &Path,
    uploads: &[String],
) -> Vec<String> {
    let mut args = vec![
        "sandbox".to_owned(),
        "create".to_owned(),
        "--name".to_owned(),
        name.to_owned(),
        "--policy".to_owned(),
        policy.to_string_lossy().into_owned(),
        "--detach".to_owned(),
        "--no-auto-providers".to_owned(),
    ];
    if !spec.image.is_empty() {
        args.push("--from".into());
        args.push(spec.image.clone());
    }
    if let Some(cpus) = spec.cpus {
        args.push("--cpu".into());
        args.push(cpus.to_string());
    }
    if let Some(memory) = &spec.memory {
        args.push("--memory".into());
        args.push(memory.clone());
    }
    for upload in uploads {
        args.push("--upload".into());
        args.push(upload.clone());
    }
    args
}

/// An OpenShell policy (schema v1) whose network section is the run's
/// egress rules. Every endpoint is enforced, on 443; the office's API host
/// is allowed on its port for the permission hook. Filesystem and process
/// settings are left to the gateway's defaults.
pub fn policy_yaml(egress: &Egress, office_port: Option<u16>) -> String {
    let mut out = String::from("version: 1\nnetwork_policies:\n");
    if let Some(port) = office_port {
        out.push_str("  shiioo_office:\n    endpoints:\n");
        out.push_str(&endpoint("host.docker.internal", port));
        out.push_str("    binaries:\n      - path: \"/**\"\n");
    }
    let hosts: Vec<String> = match egress {
        Egress::Allowlist { hosts } => hosts.iter().filter(|h| !h.is_empty()).cloned().collect(),
        Egress::Open => vec!["*".to_owned()],
    };
    if !hosts.is_empty() {
        out.push_str("  egress:\n    endpoints:\n");
        for host in &hosts {
            out.push_str(&endpoint(host, 443));
        }
        out.push_str("    binaries:\n      - path: \"/**\"\n");
    }
    out
}

fn endpoint(host: &str, port: u16) -> String {
    format!(
        "      - host: \"{host}\"\n        port: {port}\n        protocol: tcp\n        enforcement: enforce\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_lists_the_office_and_every_allowed_host() {
        let yaml = policy_yaml(
            &Egress::Allowlist {
                hosts: vec!["github.com".into(), "*.github.com".into()],
            },
            Some(7878),
        );
        assert!(yaml.starts_with("version: 1\nnetwork_policies:\n  shiioo_office:\n"));
        assert!(yaml.contains("- host: \"host.docker.internal\"\n        port: 7878"));
        assert!(yaml.contains("- host: \"*.github.com\"\n        port: 443"));
        assert_eq!(yaml.matches("enforcement: enforce").count(), 3);
    }

    #[test]
    fn create_args_carry_policy_uploads_and_limits() {
        let spec = OpenshellSpec {
            image: "ghcr.io/x/agent:1".into(),
            cpus: Some(2.0),
            memory: Some("4Gi".into()),
            egress: Egress::Open,
        };
        let args = create_args(
            "shiioo-abcd1234",
            &spec,
            Path::new("/p/policy.yaml"),
            &["/w:/workspace".to_owned()],
        );
        let s = args.join(" ");
        assert!(s.starts_with("sandbox create --name shiioo-abcd1234 --policy /p/policy.yaml --detach --no-auto-providers"));
        assert!(s.contains("--from ghcr.io/x/agent:1"));
        assert!(s.contains("--cpu 2 --memory 4Gi"));
        assert!(s.ends_with("--upload /w:/workspace"));
    }

    #[test]
    fn exec_wraps_the_harness_with_its_environment() {
        let sb = OpenshellSandbox {
            bin: "openshell".into(),
            name: "shiioo-abcd1234".into(),
            image: String::new(),
            policy: "/p/policy.yaml".into(),
            env: vec![("GH_TOKEN".into(), "x".into())],
            program: Some("codex".into()),
            egress: Egress::Open,
            hooks: None,
            created: false,
        };
        let mut host = Command::new("/opt/homebrew/bin/codex");
        host.arg("exec").arg("--json").env("SHIIOO_RUN_ID", "r1");
        let exec = sb.wrap(host);
        let args: Vec<String> = exec
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "sandbox",
                "exec",
                "-n",
                "shiioo-abcd1234",
                "--workdir",
                "/workspace",
                "--env",
                "SHIIOO_RUN_ID=r1",
                "--env",
                "GH_TOKEN=x",
                "--",
                "codex",
                "exec",
                "--json"
            ]
        );
        assert_eq!(sb.detail()["egress"]["mode"], "open");
    }
}
