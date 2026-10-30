//! The `container` isolation level against a real Docker (or Podman)
//! daemon, with a fake harness inside a stock `alpine` image.
//!
//! Ignored by default: `cargo test -p shiioo-runtime --test container --
//! --ignored` runs them where an engine is available (they pull
//! `alpine:3` and build the egress image on first use).

use shiioo_core::{EventStore, ManualClock, Payload, RunId, Seq, verify_chain};
use shiioo_runtime::{
    AgentSpec, ContainerSpec, Egress, HarnessSpec, Isolation, LocalMachine, Mount, RunOutcome,
    RunSpec, Runtime, Toolbox, UnconfiguredProvider, Worker,
};
use shiioo_store::MemoryStore;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/claude-code-read.jsonl"
);

struct Office {
    store: Arc<MemoryStore<ManualClock>>,
    machine: Arc<LocalMachine>,
    data: tempfile::TempDir,
    repo: PathBuf,
}

/// Temporary directories live under the home directory: with colima or
/// Docker Desktop only shared paths can be bind-mounted, and the system
/// temp directory is not one of them. `SHIIOO_TEST_DIR` overrides it.
fn test_dir() -> tempfile::TempDir {
    let base = std::env::var_os("SHIIOO_TEST_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/shiioo/tests")))
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&base).unwrap_or_else(|e| panic!("{e}"));
    tempfile::tempdir_in(base).unwrap_or_else(|e| panic!("{e}"))
}

impl Office {
    fn new() -> Self {
        let data = test_dir();
        let repo = data.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap_or_else(|e| panic!("{e}"));
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "user.name", "t"],
        ] {
            git(&repo, &args);
        }
        std::fs::write(repo.join("README.md"), "first line\n").unwrap_or_else(|e| panic!("{e}"));
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        let machine = LocalMachine::in_memory(data.path().join("workspace"), Toolbox::default())
            .with_worktrees(data.path().join("worktrees"))
            .with_sandboxes(data.path().join("sandboxes"));
        std::fs::create_dir_all(data.path().join("workspace")).unwrap_or_else(|e| panic!("{e}"));
        Self {
            store: Arc::new(MemoryStore::new(ManualClock::at(1_700_000_000_000))),
            machine: Arc::new(machine),
            data,
            repo,
        }
    }

    /// Writes a fake harness script into a directory the container gets
    /// mounted read-write (the script records its cwd there).
    fn fake(&self, body: &str) -> (PathBuf, PathBuf) {
        let dir = self.data.path().join("fake");
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let script = dir.join("claude");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\npwd > '{}/cwd.txt'\nenv > '{}/env.txt'\n{body}\n",
                dir.display(),
                dir.display()
            ),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        (dir, script)
    }

    fn worker(&self) -> Worker<MemoryStore<ManualClock>> {
        Worker::new(
            Arc::clone(&self.store),
            Arc::new(UnconfiguredProvider),
            Arc::clone(&self.machine),
            "w1",
        )
        .with_secrets(Arc::new(BTreeMap::from([(
            "gh-token".to_owned(),
            "ghp_testsecretvalue".to_owned(),
        )])))
    }

    async fn start(&self, script: &Path, image: &str, mounts: Vec<Mount>, egress: Egress) -> RunId {
        let mut harness = HarnessSpec::claude_code();
        harness.binary = Some(PathBuf::from("/bin/sh"));
        harness.binary_args = vec![script.to_string_lossy().into_owned()];
        harness.repo = Some(self.repo.clone());
        harness.env = BTreeMap::from([("GH_TOKEN".to_owned(), "secret:gh-token".to_owned())]);
        harness.isolation = Isolation::Container(ContainerSpec {
            image: image.into(),
            mounts,
            egress,
            ..ContainerSpec::default()
        });
        let spec = RunSpec {
            task: "Read README.md and report its first line.".into(),
            agent: AgentSpec {
                model: String::new(),
                system: String::new(),
                tools: vec!["Read".into()],
                max_turns: 10,
            },
            runtime: Runtime::Harness(harness),
            machine: None,
        };
        self.worker()
            .create(&spec)
            .await
            .unwrap_or_else(|e| panic!("{e}"))
    }

    async fn payloads(&self, run: RunId) -> Vec<Payload> {
        let log = self
            .store
            .read(run, Seq(0))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(verify_chain(&log), Ok(()));
        log.iter().map(|e| (*e.payload).clone()).collect()
    }
}

fn git(repo: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(status.success(), "git {args:?}");
}

fn containers_named(prefix: &str) -> Vec<String> {
    let out = std::process::Command::new("docker")
        .args(["ps", "-a", "--format", "{{.Names}}"])
        .output()
        .unwrap_or_else(|e| panic!("{e}"));
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|n| n.starts_with(prefix))
        .map(str::to_owned)
        .collect()
}

/// A fake harness in `alpine` sees the worktree at its host path, the
/// injected secret in its environment, and ends like Claude Code; the
/// record names the sandbox and redacts the secret; the container is gone
/// afterwards.
#[tokio::test]
#[ignore = "needs a running Docker daemon"]
async fn runs_the_harness_in_a_container_with_the_worktree_mounted() {
    let office = Office::new();
    let fixtures = PathBuf::from(FIXTURE)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    // Prints the recorded session, then leaks the secret in a result line,
    // which the redactor must catch.
    let (fake_dir, script) = office.fake(&format!(
        "head -n 4 '{FIXTURE}'\n\
         printf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\
         \"result\":\"first line; token was '\"$GH_TOKEN\"'\",\"session_id\":\"s1\",\
         \"total_cost_usd\":0,\"usage\":{{}},\"permission_denials\":[]}}'"
    ));
    let run = office
        .start(
            &script,
            "alpine:3",
            vec![
                Mount {
                    host: fake_dir.clone(),
                    readonly: false,
                },
                Mount {
                    host: fixtures,
                    readonly: true,
                },
            ],
            Egress::Open,
        )
        .await;
    let outcome = office
        .worker()
        .drive(run)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        outcome,
        RunOutcome::Completed("first line; token was [secret:gh-token]".into())
    );

    let p = office.payloads(run).await;
    let sandbox = p.iter().find_map(|x| match x {
        Payload::SandboxStarted { level, detail } => Some((level.clone(), detail.clone())),
        _ => None,
    });
    let (level, detail) = sandbox.unwrap_or_else(|| panic!("no SandboxStarted in {p:?}"));
    assert_eq!(level, "container");
    assert_eq!(detail["image"], "alpine:3");
    assert_eq!(detail["egress"]["mode"], "open");
    assert!(
        p.iter()
            .any(|x| matches!(x, Payload::SecretRedacted { name, count: 1 } if name == "gh-token")),
        "{p:?}"
    );
    assert!(
        p.iter()
            .any(|x| matches!(x, Payload::HarnessSessionStarted { .. }))
    );

    // The worktree was the working directory, at its host path.
    let worktree = office.data.path().join("worktrees").join(run.to_string());
    let cwd = std::fs::read_to_string(fake_dir.join("cwd.txt")).unwrap_or_default();
    assert_eq!(cwd.trim(), worktree.to_string_lossy());
    let env = std::fs::read_to_string(fake_dir.join("env.txt")).unwrap_or_default();
    assert!(env.contains("GH_TOKEN=ghp_testsecretvalue"), "{env}");
    assert!(env.contains("HOME=/root"), "{env}");
    // The container is gone, and so is the env file with the secret.
    assert!(containers_named(&format!("shiioo-{}", &run.to_string()[..8])).is_empty());
    assert!(
        !office
            .data
            .path()
            .join("sandboxes")
            .join(run.to_string())
            .join("env")
            .exists()
    );
}

/// With an allow list, the harness reaches an allowed host and the office
/// through the proxy and nothing else: the internal network has no other
/// way out. Uses Shiioo's harness image, which has `curl` (BusyBox's
/// `wget` cannot tunnel TLS through a proxy), and a tiny HTTP server in
/// the test standing in for the office on the host's loopback.
#[tokio::test]
#[ignore = "needs a running Docker daemon and network access"]
async fn egress_allowlist_blocks_everything_else() {
    let office = Office::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let port = listener.local_addr().map(|a| a.port()).unwrap_or_default();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                // Whatever curl sent is irrelevant; one read drains the
                // request line before answering.
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\noffice",
                    )
                    .await
                    .ok();
            });
        }
    });
    let fixtures = PathBuf::from(FIXTURE)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let (fake_dir, script) = office.fake(&format!(
        "head -n 1 '{FIXTURE}'\n\
         if curl -sSf --max-time 15 https://api.github.com/zen >/dev/null 2>&1; then a=ok; else a=blocked; fi\n\
         if curl -sSf --max-time 15 https://example.com/ >/dev/null 2>&1; then b=ok; else b=blocked; fi\n\
         c=$(curl -sS --max-time 10 http://host.docker.internal:{port}/ 2>/dev/null || echo unreachable)\n\
         printf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\
         \"result\":\"github:'$a' example:'$b' office:'$c'\",\"session_id\":\"s1\",\
         \"total_cost_usd\":0,\"usage\":{{}},\"permission_denials\":[]}}'"
    ));
    let image = shiioo_runtime::sandbox::container::default_image();
    let run = office
        .start(
            &script,
            &image,
            vec![
                Mount {
                    host: fake_dir,
                    readonly: false,
                },
                Mount {
                    host: fixtures,
                    readonly: true,
                },
            ],
            Egress::Allowlist {
                hosts: vec!["api.github.com".into()],
            },
        )
        .await;
    let outcome = office
        .worker()
        .drive(run)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        outcome,
        RunOutcome::Completed("github:ok example:blocked office:office".into())
    );
    let p = office.payloads(run).await;
    let detail = p
        .iter()
        .find_map(|x| match x {
            Payload::SandboxStarted { detail, .. } => Some(detail.clone()),
            _ => None,
        })
        .unwrap_or_default();
    assert_eq!(detail["egress"]["mode"], "allowlist");
    assert_eq!(detail["egress"]["hosts"][0], "api.github.com");
    assert!(containers_named(&format!("shiioo-egress-{}", &run.to_string()[..8])).is_empty());
}
