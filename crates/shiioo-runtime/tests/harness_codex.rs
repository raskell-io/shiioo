//! Codex as a harness, driven through a fake `codex` that replays recorded
//! output (`tests/fixtures/codex-command.jsonl`).

use shiioo_core::{EventStore, ManualClock, Payload, RunId, Seq, verify_chain};
use shiioo_runtime::{
    AgentSpec, HarnessSpec, LocalMachine, RunOutcome, RunSpec, Runtime, Toolbox,
    UnconfiguredProvider, Worker, branch_for,
};
use shiioo_store::MemoryStore;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const TTL: u64 = 30_000;
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/codex-command.jsonl"
);
const THREAD: &str = "01a17237-e570-7033-8077-8e667f293b33";
const FINAL: &str = "Sun Oct 25 09:40:34 CET 2026";

struct Fake {
    dir: tempfile::TempDir,
}

impl Fake {
    /// A `codex` that logs its arguments and cwd, replays the fixture
    /// (hanging after `hang_after` lines, if set), and replays a short
    /// completion when asked to `resume`. `--version` prints a version.
    fn new(hang_after: Option<usize>) -> Self {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let text = std::fs::read_to_string(FIXTURE).unwrap_or_else(|e| panic!("{e}"));
        let lines: Vec<&str> = text.lines().collect();
        let resume = [lines[0], lines[5], lines[6]].join("\n") + "\n";
        std::fs::write(dir.path().join("resume.jsonl"), resume).unwrap_or_else(|e| panic!("{e}"));
        let hang = match hang_after {
            Some(n) => format!("head -n {n} '{FIXTURE}'; sleep 60; exit 1"),
            None => format!("cat '{FIXTURE}'"),
        };
        let script = format!(
            "#!/bin/sh\n\
             if [ \"$1\" = --version ]; then echo 'codex-cli 0.163.0'; exit 0; fi\n\
             printf '%s\\n' \"$@\" > '{args}'\npwd >> '{args}'\n\
             case \" $* \" in *' resume '*) cat '{resume}'; exit 0;; esac\n{hang}\n",
            args = dir.path().join("args.txt").display(),
            resume = dir.path().join("resume.jsonl").display(),
        );
        let bin = dir.path().join("codex");
        std::fs::write(&bin, script).unwrap_or_else(|e| panic!("{e}"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|e| panic!("{e}"));
        }
        Self { dir }
    }

    fn binary(&self) -> PathBuf {
        self.dir.path().join("codex")
    }

    fn args(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("args.txt"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

struct Fixture {
    store: Arc<MemoryStore<ManualClock>>,
    clock: ManualClock,
    machine: Arc<LocalMachine>,
    data: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let data = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let clock = ManualClock::at(1_700_000_000_000);
        let machine = LocalMachine::in_memory(data.path().join("workspace"), Toolbox::default())
            .with_worktrees(data.path().join("worktrees"));
        std::fs::create_dir_all(data.path().join("workspace")).unwrap_or_else(|e| panic!("{e}"));
        Self {
            store: Arc::new(MemoryStore::new(clock.clone())),
            clock,
            machine: Arc::new(machine),
            data,
        }
    }

    fn worker(&self, id: &str) -> Worker<MemoryStore<ManualClock>> {
        Worker::new(
            Arc::clone(&self.store),
            Arc::new(UnconfiguredProvider),
            Arc::clone(&self.machine),
            id,
        )
    }

    async fn start(&self, fake: &Fake, repo: Option<PathBuf>) -> RunId {
        let mut harness = HarnessSpec::codex();
        harness.binary = Some(PathBuf::from("/bin/sh"));
        harness.binary_args = vec![fake.binary().to_string_lossy().into_owned()];
        harness.repo = repo;
        let spec = RunSpec {
            task: "Run date and report its output.".into(),
            agent: AgentSpec {
                model: String::new(),
                system: String::new(),
                tools: vec![],
                max_turns: 10,
            },
            runtime: Runtime::Harness(harness),
            machine: None,
        };
        self.worker("creator")
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

fn count(payloads: &[Payload], pred: impl Fn(&Payload) -> bool) -> usize {
    payloads.iter().filter(|p| pred(p)).count()
}

#[tokio::test]
async fn records_a_codex_thread_as_events() {
    let fake = Fake::new(None);
    let fx = Fixture::new();
    let run = fx.start(&fake, None).await;

    let outcome = fx
        .worker("w1")
        .drive(run)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(outcome, RunOutcome::Completed(FINAL.into()));

    let p = fx.payloads(run).await;
    assert_eq!(
        count(
            &p,
            |x| matches!(x, Payload::HarnessSessionStarted { harness, session_id, version, .. }
            if harness == "codex" && session_id == THREAD && version == "0.163.0")
        ),
        1
    );
    assert_eq!(
        count(&p, |x| matches!(x, Payload::ModelResponseRecorded { .. })),
        2
    );
    assert_eq!(
        count(&p, |x| matches!(x, Payload::ToolIntent { tool, input, .. }
            if tool == "Bash" && input["command"] == "/bin/zsh -lc date")),
        1,
        "one intent, even though Codex reports the command twice (started, completed)"
    );
    assert_eq!(
        count(
            &p,
            |x| matches!(x, Payload::ToolResult { output, is_error: false, .. }
            if output.as_str().is_some_and(|o| o.starts_with(FINAL)))
        ),
        1
    );
    assert!(matches!(
        &p[p.len() - 2],
        Payload::HarnessResult { harness, subtype, cost_usd: None, usage, .. }
            if harness == "codex" && subtype == "success" && usage["output_tokens"] == 64
    ));
    assert!(matches!(p.last(), Some(Payload::RunCompleted { .. })));

    let args = fake.args();
    assert_eq!(args.first().map(String::as_str), Some("exec"));
    assert!(args.contains(&"--json".to_string()));
    assert!(
        args.contains(&"--skip-git-repo-check".to_string()),
        "no repo: {args:?}"
    );
    // Default: Codex's own sandbox is off; Shiioo's rules govern commands.
    assert!(
        args.contains(&"--dangerously-bypass-approvals-and-sandbox".to_string())
            || args.contains(&"sandbox_mode=\"danger-full-access\"".to_string()),
        "{args:?}"
    );
    assert!(!args.iter().any(|a| a == "resume"));
    // The prompt is the last argument (the line after it is the cwd).
    assert_eq!(
        args.get(args.len() - 2).map(String::as_str),
        Some("Run date and report its output.")
    );
}

/// The worker dies while Codex is mid-command. The next worker resumes
/// the same Codex thread instead of starting over.
#[tokio::test]
async fn killed_worker_resumes_the_codex_thread() {
    let fake = Fake::new(Some(4));
    let fx = Fixture::new();
    let run = fx.start(&fake, None).await;

    // Run w1 until it has recorded the thread, then kill its task while the
    // harness is still hung. Waiting on the log keeps this deterministic.
    let w1 = Arc::new(fx.worker("w1"));
    let task = {
        let w1 = Arc::clone(&w1);
        tokio::spawn(async move {
            let _ = w1.drive(run).await;
        })
    };
    wait_for_sessions(&fx.store, run, 1).await;
    task.abort();
    task.await.ok();

    fx.clock.advance_ms(TTL + 1);
    let outcome = fx
        .worker("w2")
        .drive(run)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(outcome, RunOutcome::Completed(FINAL.into()));

    let args = fake.args();
    let resume_at = args.iter().position(|a| a == "resume");
    assert_eq!(
        resume_at.and_then(|i| args.get(i + 1)).map(String::as_str),
        Some(THREAD),
        "{args:?}"
    );
    let p = fx.payloads(run).await;
    assert_eq!(
        count(&p, |x| matches!(x, Payload::HarnessSessionStarted { .. })),
        2
    );
    assert_eq!(count(&p, |x| matches!(x, Payload::LeaseAcquired { .. })), 2);
    // The interrupted command's intent is in the log; its result never came.
    assert_eq!(count(&p, |x| matches!(x, Payload::ToolIntent { .. })), 1);
    assert!(matches!(p.last(), Some(Payload::RunCompleted { .. })));
}

/// A run on a repository gets its worktree, and Codex is pointed at it.
#[tokio::test]
async fn repo_runs_point_codex_at_the_worktree() {
    let fake = Fake::new(None);
    let fx = Fixture::new();
    let repo = fx.data.path().join("repo");
    git_init_with_commit(&repo).await;

    let run = fx.start(&fake, Some(repo.clone())).await;
    let outcome = fx
        .worker("w1")
        .drive(run)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(outcome, RunOutcome::Completed(_)));

    let worktree = fx.data.path().join("worktrees").join(run.to_string());
    assert!(worktree.join(".git").exists());
    let branches = git(&repo, &["branch", "--list", &branch_for(run)]).await;
    assert!(branches.contains(&branch_for(run)), "{branches}");

    let args = fake.args();
    assert!(!args.contains(&"--skip-git-repo-check".to_string()));
    let c_at = args.iter().position(|a| a == "-C");
    let pointed = c_at.and_then(|i| args.get(i + 1)).map(PathBuf::from);
    assert_eq!(
        pointed.and_then(|p| p.canonicalize().ok()),
        worktree.canonicalize().ok(),
        "-C must name the worktree"
    );
    let cwd = args.last().map(PathBuf::from).unwrap_or_default();
    assert_eq!(cwd.canonicalize().ok(), worktree.canonicalize().ok());
}

async fn git_init_with_commit(repo: &Path) {
    std::fs::create_dir_all(repo).unwrap_or_else(|e| panic!("{e}"));
    git(repo, &["init", "-q", "-b", "main"]).await;
    std::fs::write(repo.join("README.md"), "first line\n").unwrap_or_else(|e| panic!("{e}"));
    git(repo, &["add", "."]).await;
    git(
        repo,
        &[
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-q",
            "-m",
            "init",
        ],
    )
    .await;
}

async fn git(repo: &Path, args: &[&str]) -> String {
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Polls the run's log until it holds at least `n` `HarnessSessionStarted`
/// events, so a test can act on a running harness without racing a clock.
async fn wait_for_sessions(store: &MemoryStore<ManualClock>, run: RunId, n: usize) {
    for _ in 0..400 {
        let started = store
            .read(run, Seq(0))
            .await
            .unwrap_or_default()
            .iter()
            .filter(|e| matches!(*e.payload, Payload::HarnessSessionStarted { .. }))
            .count();
        if started >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the harness never recorded {n} session(s) within 10s");
}
