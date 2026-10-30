//! The `container` level: one OCI container per run, through the Docker
//! (or Podman) CLI the owner already has.
//!
//! What the container gets: a read-only root filesystem, no capabilities,
//! `no-new-privileges`, a pid limit, optional CPU and memory limits; the
//! run's worktree and the repository's `.git` directory bind-mounted at
//! their host paths (so the worktree's `.git` pointer resolves); a home
//! directory under `.shiioo/sandboxes/<run>/home` that persists across
//! resumes, where Claude Code and Codex keep their sessions; credentials
//! for the harness (Codex's `auth.json` read-only, Claude Code's OAuth
//! token in the environment); and, unless egress is `open`, a place on the
//! internal network with the proxy of [`super::egress`] as its only way out.
//!
//! The process inside runs as root. The user id does not matter for
//! isolation (root in a container with no capabilities is an ordinary
//! user to the host), and bind mounts under colima are only writable by
//! root. Files it creates show up on the host as the owner's.

use super::egress::{self, Proxy};
use super::{SandboxContext, SandboxError, hooks_from_inside, output, short_hash, short_run};
use crate::harness::claude_code::HookEndpoint;
use crate::secrets::CLAUDE_OAUTH_TOKEN;
use crate::spec::{ContainerSpec, Egress, HarnessKind};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

pub const DOCKERFILE: &str = include_str!("harness.Dockerfile");

/// Shiioo's own harness image, tagged by its Dockerfile.
pub fn default_image() -> String {
    format!("shiioo-harness:{}", short_hash(DOCKERFILE))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Bind {
    host: PathBuf,
    container: PathBuf,
    readonly: bool,
}

pub struct ContainerSandbox {
    engine: String,
    image: String,
    name: String,
    dir: PathBuf,
    cwd: PathBuf,
    binds: Vec<Bind>,
    env: Vec<(String, String)>,
    memory: Option<String>,
    cpus: Option<f64>,
    proxy: Option<Proxy>,
    /// The harness the image provides, used instead of the host's binary
    /// path unless the spec named one.
    program: Option<String>,
    pub(crate) hooks: Option<HookEndpoint>,
    git_https: Vec<String>,
}

impl ContainerSandbox {
    pub async fn prepare(
        spec: &ContainerSpec,
        ctx: &SandboxContext<'_>,
    ) -> Result<Self, SandboxError> {
        let engine = if spec.engine.is_empty() {
            "docker".to_owned()
        } else {
            spec.engine.clone()
        };
        let root = ctx.sandboxes_root.ok_or_else(|| {
            SandboxError::Config("this machine has no sandbox directory for container runs".into())
        })?;
        check_engine(&engine).await?;
        let dir = root.join(ctx.run_id.to_string());
        let home = dir.join("home");
        std::fs::create_dir_all(home.join(".codex"))?;
        std::fs::create_dir_all(home.join(".claude"))?;

        let mut env = ctx.env.to_vec();
        let mut binds = vec![
            Bind {
                host: ctx.cwd.to_path_buf(),
                container: ctx.cwd.to_path_buf(),
                readonly: false,
            },
            Bind {
                host: home.clone(),
                container: PathBuf::from("/root"),
                readonly: false,
            },
        ];
        if let Some(repo) = ctx.repo
            && repo != ctx.cwd
        {
            binds.push(Bind {
                host: repo.join(".git"),
                container: repo.join(".git"),
                readonly: false,
            });
        }
        for m in &spec.mounts {
            binds.push(Bind {
                host: m.host.clone(),
                container: m.host.clone(),
                readonly: m.readonly,
            });
        }
        // A spec that names its own binary (a wrapper, a test double)
        // brings its own login; the image's harnesses need the owner's.
        if ctx.binary_override.is_none() {
            credentials(ctx, &mut env, &mut binds)?;
        }

        let image = if spec.image.is_empty() {
            let image = default_image();
            ensure_image(&engine, &image, DOCKERFILE, &dir).await?;
            image
        } else {
            spec.image.clone()
        };
        check_mountable(&engine, &image, ctx.cwd).await?;
        let short = short_run(ctx.run_id);
        let proxy = match &spec.egress {
            Egress::Allowlist { hosts } => {
                Some(egress::start(&engine, &dir.join("egress"), &short, hosts).await?)
            }
            Egress::Open => None,
        };
        let name = format!("shiioo-{short}");
        remove_container(&engine, &name).await;
        Ok(Self {
            engine,
            image,
            name,
            dir,
            cwd: ctx.cwd.to_path_buf(),
            binds,
            env,
            memory: spec.memory.clone(),
            cpus: spec.cpus,
            proxy,
            program: ctx
                .binary_override
                .is_none()
                .then(|| image_binary(ctx.harness).to_owned()),
            hooks: hooks_from_inside(ctx.hooks),
            git_https: spec.git_https.clone(),
        })
    }

    /// `docker run …` for the host command. Environment goes through a
    /// 0600 env file in the run's sandbox directory, not the command line.
    pub fn wrap(&self, cmd: Command) -> Command {
        let std_cmd = cmd.as_std();
        let program = self
            .program
            .clone()
            .unwrap_or_else(|| std_cmd.get_program().to_string_lossy().into_owned());
        let mut env: Vec<(String, String)> = std_cmd
            .get_envs()
            .filter_map(|(k, v)| {
                Some((
                    k.to_string_lossy().into_owned(),
                    v?.to_string_lossy().into_owned(),
                ))
            })
            .collect();
        env.extend(self.env.iter().cloned());
        env.push(("HOME".into(), "/root".into()));
        if let Some(proxy) = &self.proxy {
            env.extend(egress::proxy_env(&proxy.url));
        }
        env.extend(git_config_env(&self.git_https));
        let env_file = self.dir.join("env");
        write_env_file(&env_file, &env);

        let mut docker = Command::new(&self.engine);
        docker.args([
            "run",
            "--rm",
            "--name",
            &self.name,
            "--hostname",
            "shiioo",
            "--read-only",
            "--tmpfs",
            "/tmp:rw,exec,size=4g",
            "--tmpfs",
            "/run:rw,size=64m",
            "--cap-drop",
            "ALL",
            "--security-opt",
            "no-new-privileges",
            "--pids-limit",
            "2048",
            "--add-host",
            "host.docker.internal:host-gateway",
        ]);
        if let Some(memory) = &self.memory {
            docker.arg("--memory").arg(memory);
        }
        if let Some(cpus) = self.cpus {
            docker.arg("--cpus").arg(cpus.to_string());
        }
        for bind in &self.binds {
            let mut spec = format!(
                "type=bind,src={},dst={}",
                bind.host.to_string_lossy(),
                bind.container.to_string_lossy()
            );
            if bind.readonly {
                spec.push_str(",readonly");
            }
            docker.arg("--mount").arg(spec);
        }
        if self.proxy.is_some() {
            docker.arg("--network").arg(egress::NETWORK);
        }
        docker
            .arg("-w")
            .arg(&self.cwd)
            .arg("--env-file")
            .arg(&env_file)
            .arg(&self.image)
            .arg(program)
            .args(std_cmd.get_args());
        docker
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        docker
    }

    pub fn detail(&self) -> Value {
        json!({
            "engine": self.engine,
            "image": self.image,
            "container": self.name,
            "cwd": self.cwd.to_string_lossy(),
            "mounts": self.binds.iter().map(|b| json!({
                "path": b.container.to_string_lossy(),
                "readonly": b.readonly,
            })).collect::<Vec<_>>(),
            "egress": match &self.proxy {
                Some(p) => json!({ "mode": "allowlist", "proxy": p.name, "hosts": p.hosts }),
                None => json!({ "mode": "open" }),
            },
            "memory": self.memory,
            "cpus": self.cpus,
            "env": self.env.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
        })
    }

    /// Kills the harness container (the `docker run` client dying does not
    /// stop it) and the run's proxy. Removes the env file with the secrets.
    pub async fn teardown(&self) {
        remove_container(&self.engine, &self.name).await;
        if let Some(proxy) = &self.proxy {
            remove_container(&self.engine, &proxy.name).await;
        }
        std::fs::remove_file(self.dir.join("env")).ok();
    }
}

impl Drop for ContainerSandbox {
    /// Last resort if the worker is dropped without a teardown: ask the
    /// engine to remove the containers, without waiting.
    fn drop(&mut self) {
        let mut names = vec![self.name.clone()];
        if let Some(proxy) = &self.proxy {
            names.push(proxy.name.clone());
        }
        std::process::Command::new(&self.engine)
            .arg("rm")
            .arg("-f")
            .args(&names)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok();
    }
}

/// The harness as installed in the image.
fn image_binary(harness: HarnessKind) -> &'static str {
    match harness {
        HarnessKind::ClaudeCode => "claude",
        HarnessKind::Codex => "codex",
    }
}

/// What the harness needs to log in, taken from the host: Codex's auth
/// file, mounted read-only; Claude Code's OAuth token from the office's
/// secrets (or this process's environment), because on macOS its login
/// lives in the keychain, which a container cannot read.
fn credentials(
    ctx: &SandboxContext<'_>,
    env: &mut Vec<(String, String)>,
    binds: &mut Vec<Bind>,
) -> Result<(), SandboxError> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    match ctx.harness {
        HarnessKind::Codex => {
            let auth = std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex"))
                .join("auth.json");
            if !auth.is_file() {
                return Err(SandboxError::Credentials(format!(
                    "codex is not logged in on this machine ({} missing); run `codex login`",
                    auth.display()
                )));
            }
            binds.push(Bind {
                host: auth,
                container: PathBuf::from("/root/.codex/auth.json"),
                readonly: true,
            });
        }
        HarnessKind::ClaudeCode => {
            if env.iter().any(|(k, _)| k == "CLAUDE_CODE_OAUTH_TOKEN") {
                return Ok(());
            }
            let token = ctx
                .secrets
                .get(CLAUDE_OAUTH_TOKEN)
                .or_else(|| std::env::var("CLAUDE_CODE_OAUTH_TOKEN").ok())
                .filter(|t| !t.is_empty());
            if let Some(token) = token {
                env.push(("CLAUDE_CODE_OAUTH_TOKEN".into(), token));
                return Ok(());
            }
            let creds = home.join(".claude").join(".credentials.json");
            if creds.is_file() {
                binds.push(Bind {
                    host: creds,
                    container: PathBuf::from("/root/.claude/.credentials.json"),
                    readonly: true,
                });
                return Ok(());
            }
            return Err(SandboxError::Credentials(format!(
                "no Claude Code login a container can use: run `claude setup-token` and store \
                 the token with `shiioo secret set {CLAUDE_OAUTH_TOKEN}`"
            )));
        }
    }
    Ok(())
}

/// `GIT_CONFIG_*` variables: `insteadOf` rewrites for the owner's SSH
/// remote aliases, which have no keys inside the container.
pub(crate) fn git_config_env(git_https: &[String]) -> Vec<(String, String)> {
    let mut env = Vec::new();
    let prefixes: Vec<&str> = git_https
        .iter()
        .map(String::as_str)
        .filter(|p| !p.is_empty())
        .collect();
    if prefixes.is_empty() {
        return env;
    }
    env.push(("GIT_CONFIG_COUNT".into(), prefixes.len().to_string()));
    for (i, prefix) in prefixes.iter().enumerate() {
        env.push((
            format!("GIT_CONFIG_KEY_{i}"),
            "url.https://github.com/.insteadOf".into(),
        ));
        env.push((format!("GIT_CONFIG_VALUE_{i}"), (*prefix).to_owned()));
    }
    env
}

/// Writes `K=V` lines, mode 0600. Values are written verbatim (Docker's
/// env-file takes no quoting), so a value with a newline is truncated at
/// it.
fn write_env_file(path: &Path, env: &[(String, String)]) {
    let body: String = env
        .iter()
        .map(|(k, v)| format!("{k}={}\n", v.lines().next().unwrap_or_default()))
        .collect();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(path, body).ok();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).ok();
    }
}

/// Fails with a clear message when the engine is missing or its daemon is
/// not running.
pub async fn check_engine(engine: &str) -> Result<(), SandboxError> {
    output(engine, &["version", "--format", "{{.Server.Version}}"])
        .await
        .map(|_| ())
        .map_err(|e| match e {
            SandboxError::Command { stderr, .. } => SandboxError::Unavailable(format!(
                "{engine} is installed but not running ({})",
                stderr.lines().next().unwrap_or_default()
            )),
            other => other,
        })
}

/// Builds `image` from `dockerfile` unless it exists. The build reads the
/// Dockerfile from stdin with `dir` as an (empty) context.
pub async fn ensure_image(
    engine: &str,
    image: &str,
    dockerfile: &str,
    dir: &Path,
) -> Result<(), SandboxError> {
    if output(engine, &["image", "inspect", image]).await.is_ok() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    eprintln!("sandbox: building {image} (first use; this takes a few minutes)");
    let mut child = Command::new(engine)
        .args(["build", "-t", image, "-f", "-"])
        .arg(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| SandboxError::Unavailable(format!("{engine} is not available ({e})")))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(dockerfile.as_bytes()).await?;
        drop(stdin);
    }
    let out = child.wait_with_output().await?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(15).collect();
        return Err(SandboxError::Command {
            program: engine.into(),
            args: format!("build -t {image}"),
            stderr: tail.into_iter().rev().collect::<Vec<_>>().join("\n"),
        });
    }
    eprintln!("sandbox: built {image}");
    Ok(())
}

/// Fails with a usable message when the engine cannot bind-mount `path`.
/// Under colima and Docker Desktop only shared directories (the home
/// directory by default) are visible to containers; an office or
/// repository elsewhere fails here instead of five times at harness start.
pub async fn check_mountable(engine: &str, image: &str, path: &Path) -> Result<(), SandboxError> {
    let mount = format!(
        "type=bind,src={},dst=/mnt/shiioo-check,readonly",
        path.to_string_lossy()
    );
    match output(
        engine,
        &[
            "run",
            "--rm",
            "--entrypoint",
            "/bin/sh",
            "--mount",
            &mount,
            image,
            "-c",
            "true",
        ],
    )
    .await
    {
        Ok(_) => Ok(()),
        Err(SandboxError::Command { stderr, .. }) if stderr.contains("does not exist") => {
            Err(SandboxError::Config(format!(
                "{engine} cannot see {}: with colima or Docker Desktop only shared directories                  (your home directory by default) can be mounted; keep the office and its                  repositories there, or add the path to the VM's mounts",
                path.display()
            )))
        }
        Err(e) => Err(e),
    }
}

/// Removes a container if it exists; quiet otherwise.
pub async fn remove_container(engine: &str, name: &str) {
    output(engine, &["rm", "-f", name]).await.ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::claude_code::HookCommand;
    use std::path::Path;

    fn sandbox(proxy: Option<Proxy>, program: Option<&str>) -> ContainerSandbox {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        ContainerSandbox {
            engine: "docker".into(),
            image: "shiioo-harness:test".into(),
            name: "shiioo-abcd1234".into(),
            dir: dir.keep(),
            cwd: PathBuf::from("/Users/me/office/.shiioo/worktrees/r1"),
            binds: vec![
                Bind {
                    host: "/Users/me/office/.shiioo/worktrees/r1".into(),
                    container: "/Users/me/office/.shiioo/worktrees/r1".into(),
                    readonly: false,
                },
                Bind {
                    host: "/Users/me/.codex/auth.json".into(),
                    container: "/root/.codex/auth.json".into(),
                    readonly: true,
                },
            ],
            env: vec![("GH_TOKEN".into(), "ghp_x".into())],
            memory: Some("8g".into()),
            cpus: Some(4.0),
            proxy,
            program: program.map(str::to_owned),
            hooks: Some(HookEndpoint {
                server_url: "http://host.docker.internal:7878".into(),
                token: "t".into(),
                command: HookCommand::Curl,
            }),
            git_https: vec!["git@github-work:".into()],
        }
    }

    fn args_of(cmd: &Command) -> Vec<String> {
        cmd.as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn wraps_the_harness_command_into_docker_run() {
        let sb = sandbox(
            Some(Proxy {
                name: "shiioo-egress-abcd1234".into(),
                url: "http://shiioo-egress-abcd1234:3128".into(),
                hosts: vec!["github.com".into()],
            }),
            Some("claude"),
        );
        let mut host = Command::new("/opt/homebrew/bin/claude");
        host.arg("--print").arg("do it").env("SHIIOO_RUN_ID", "r1");
        let docker = sb.wrap(host);
        assert_eq!(docker.as_std().get_program(), "docker");
        let args = args_of(&docker);
        let s = args.join(" ");
        assert!(s.starts_with("run --rm --name shiioo-abcd1234"), "{s}");
        for flag in [
            "--read-only",
            "--cap-drop ALL",
            "--security-opt no-new-privileges",
            "--memory 8g",
            "--cpus 4",
            "--network shiioo-sandbox",
            "--mount type=bind,src=/Users/me/office/.shiioo/worktrees/r1,dst=/Users/me/office/.shiioo/worktrees/r1",
            "--mount type=bind,src=/Users/me/.codex/auth.json,dst=/root/.codex/auth.json,readonly",
            "-w /Users/me/office/.shiioo/worktrees/r1",
            "--add-host host.docker.internal:host-gateway",
        ] {
            assert!(s.contains(flag), "missing {flag}: {s}");
        }
        // The image's claude, not the host path, then the harness's args.
        let image_at = args.iter().position(|a| a == "shiioo-harness:test");
        assert_eq!(
            image_at.map(|i| &args[i + 1..]),
            Some(
                &[
                    "claude".to_owned(),
                    "--print".to_owned(),
                    "do it".to_owned()
                ][..]
            )
        );
        // The environment went into the env file, not the arguments.
        assert!(!s.contains("ghp_x"));
        let env_at = args.iter().position(|a| a == "--env-file");
        let file = env_at
            .and_then(|i| args.get(i + 1))
            .unwrap_or_else(|| panic!("no env file"));
        let body = std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{e}"));
        for line in [
            "SHIIOO_RUN_ID=r1",
            "GH_TOKEN=ghp_x",
            "HOME=/root",
            "HTTPS_PROXY=http://shiioo-egress-abcd1234:3128",
            "GIT_CONFIG_COUNT=1",
            "GIT_CONFIG_KEY_0=url.https://github.com/.insteadOf",
            "GIT_CONFIG_VALUE_0=git@github-work:",
        ] {
            assert!(body.contains(line), "missing {line} in {body}");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(file)
                .unwrap_or_else(|e| panic!("{e}"))
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn open_egress_uses_the_default_network_and_keeps_the_named_binary() {
        let sb = sandbox(None, None);
        let mut host = Command::new("/bin/sh");
        host.arg("/tmp/fake-claude");
        let docker = sb.wrap(host);
        let s = args_of(&docker).join(" ");
        assert!(!s.contains("--network"), "{s}");
        assert!(
            s.ends_with("shiioo-harness:test /bin/sh /tmp/fake-claude"),
            "{s}"
        );
        let detail = sb.detail();
        assert_eq!(detail["egress"]["mode"], "open");
        assert_eq!(detail["container"], "shiioo-abcd1234");
    }

    #[test]
    fn git_config_env_is_empty_without_prefixes() {
        assert!(git_config_env(&[]).is_empty());
        assert!(git_config_env(&[String::new()]).is_empty());
        let env = git_config_env(&["a:".into(), "b:".into()]);
        assert_eq!(env[0], ("GIT_CONFIG_COUNT".to_owned(), "2".to_owned()));
        assert_eq!(env.len(), 5);
    }

    #[test]
    fn default_image_tag_follows_the_dockerfile() {
        assert!(default_image().starts_with("shiioo-harness:"));
        assert!(Path::new("/").is_absolute());
    }
}
