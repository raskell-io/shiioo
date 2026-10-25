//! Claude Code as a harness, driven through a fake `claude` that replays
//! recorded output (`tests/fixtures/claude-code-read.jsonl`).

use shiioo_core::{EventStore, ManualClock, Payload, RunId, Seq, verify_chain};
use shiioo_runtime::{
    AgentSpec, HarnessSpec, LocalMachine, RunOutcome, RunSpec, Runtime, Toolbox,
    UnconfiguredProvider, Worker, WorkerError, branch_for,
};
use shiioo_store::MemoryStore;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const TTL: u64 = 30_000;
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/claude-code-read.jsonl"
);
const SESSION: &str = "a11a080d-e85c-457e-9b31-578ebfba0c1c";

struct Fake {
    dir: tempfile::TempDir,
}

impl Fake {
    /// Writes a `claude` script that logs its arguments and cwd, replays
    /// the fixture (stopping after `hang_after` lines and hanging, if set),
    /// and replays a shorter completion when resumed.
    fn new(hang_after: Option<usize>) -> Self {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let lines: Vec<&str> = std::fs::read_to_string(FIXTURE)
            .unwrap_or_else(|e| panic!("{e}"))
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>()
            .leak()
            .iter()
            .map(String::as_str)
            .collect();
        // Resuming: init again, then the final answer and the result.
        let resume = [lines[0], lines[4], lines[5]].join("\n") + "\n";
        std::fs::write(dir.path().join("resume.jsonl"), resume).unwrap_or_else(|e| panic!("{e}"));
        let hang = match hang_after {
            Some(n) => format!("head -n {n} '{FIXTURE}'; sleep 60; exit 1"),
            None => format!("cat '{FIXTURE}'"),
        };
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{args}'\npwd >> '{args}'\n\
             case \" $* \" in *' --resume '*) cat '{resume}'; exit 0;; esac\n{hang}\n",
            args = dir.path().join("args.txt").display(),
            resume = dir.path().join("resume.jsonl").display(),
        );
        let bin = dir.path().join("claude");
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
        self.dir.path().join("claude")
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
        let mut harness = HarnessSpec::claude_code();
        harness.binary = Some(PathBuf::from("/bin/sh"));
        harness.binary_args = vec![fake.binary().to_string_lossy().into_owned()];
        harness.repo = repo;
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
async fn records_a_harness_session_as_events() {
    let fake = Fake::new(None);
    let fx = Fixture::new();
    let run = fx.start(&fake, None).await;

    let outcome = fx
        .worker("w1")
        .drive(run)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        outcome,
        RunOutcome::Completed("first line of readme".into())
    );

    let p = fx.payloads(run).await;
    assert_eq!(
        count(
            &p,
            |x| matches!(x, Payload::HarnessSessionStarted { session_id, .. } if session_id == SESSION)
        ),
        1
    );
    assert_eq!(
        count(&p, |x| matches!(x, Payload::ModelResponseRecorded { .. })),
        2
    );
    assert_eq!(
        count(
            &p,
            |x| matches!(x, Payload::ToolIntent { tool, .. } if tool == "Read")
        ),
        1
    );
    assert_eq!(count(&p, |x| matches!(x, Payload::ToolResult { .. })), 1);
    assert_eq!(
        count(&p, |x| matches!(x, Payload::CapacityObserved { .. })),
        1
    );
    assert!(matches!(
        &p[p.len() - 2],
        Payload::HarnessResult { subtype, cost_usd: Some(_), .. } if subtype == "success"
    ));
    assert!(matches!(p.last(), Some(Payload::RunCompleted { .. })));

    let args = fake.args();
    assert!(args.contains(&"--print".to_string()));
    assert!(args.contains(&"stream-json".to_string()));
    assert!(args.contains(&"acceptEdits".to_string()));
    assert!(args.contains(&"Read".to_string()));
    assert!(!args.iter().any(|a| a == "--resume"));
    assert_eq!(
        args.get(1).map(String::as_str),
        Some("Read README.md and report its first line."),
        "the prompt comes right after --print: {args:?}"
    );
}

/// The worker is killed (its future dropped, as with kill -9) while the
/// harness is mid-task. The next worker resumes the same harness session
/// instead of starting the task over.
#[tokio::test]
async fn killed_worker_resumes_the_harness_session() {
    let fake = Fake::new(Some(2));
    let fx = Fixture::new();
    let run = fx.start(&fake, None).await;

    // Run w1 until it has recorded the harness session, then kill it (drop
    // its task, as with kill -9) while the harness is still hung. Waiting
    // on the log, not a wall clock, keeps this deterministic under load.
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

    // w1 never released its lease. After it expires, w2 takes over.
    fx.clock.advance_ms(TTL + 1);
    let outcome = fx
        .worker("w2")
        .drive(run)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        outcome,
        RunOutcome::Completed("first line of readme".into())
    );

    let args = fake.args();
    let resume_at = args.iter().position(|a| a == "--resume");
    assert_eq!(
        resume_at.and_then(|i| args.get(i + 1)).map(String::as_str),
        Some(SESSION),
        "{args:?}"
    );
    let p = fx.payloads(run).await;
    assert_eq!(
        count(&p, |x| matches!(x, Payload::HarnessSessionStarted { .. })),
        2
    );
    assert_eq!(count(&p, |x| matches!(x, Payload::LeaseAcquired { .. })), 2);
    assert!(matches!(p.last(), Some(Payload::RunCompleted { .. })));
}

/// A harness that dies without a result leaves the run open with a clear
/// error; the run is not failed, because the next worker can resume it.
#[tokio::test]
async fn harness_dying_leaves_the_run_open() {
    let fake = Fake::new(None);
    // A script that prints the init line and exits non-zero.
    let fixture_init = std::fs::read_to_string(FIXTURE)
        .unwrap_or_else(|e| panic!("{e}"))
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    std::fs::write(
        fake.binary(),
        format!("#!/bin/sh\nprintf '%s\\n' '{fixture_init}'\necho boom >&2\nexit 7\n"),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let fx = Fixture::new();
    let run = fx.start(&fake, None).await;

    let err = fx.worker("w1").drive(run).await;
    assert!(
        matches!(err, Err(WorkerError::Harness(shiioo_runtime::HarnessError::ExitedEarly { ref stderr, .. })) if stderr == "boom"),
        "{err:?}"
    );
    let p = fx.payloads(run).await;
    assert!(!p.iter().any(Payload::is_terminal));
}

/// A run on a repository gets its own worktree and branch, and the
/// harness runs inside that worktree.
#[tokio::test]
async fn repo_runs_get_a_worktree_and_branch() {
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
    assert!(
        worktree.join(".git").exists(),
        "worktree at {}",
        worktree.display()
    );
    assert!(
        worktree.join("README.md").exists(),
        "worktree has the repo's files"
    );
    let branches = git(&repo, &["branch", "--list", &branch_for(run)]).await;
    assert!(branches.contains(&branch_for(run)), "{branches}");

    let cwd = fake.args().last().map(PathBuf::from).unwrap_or_default();
    assert_eq!(
        cwd.canonicalize().ok(),
        worktree.canonicalize().ok(),
        "harness ran in the worktree"
    );

    // Driving again (after a crash) reuses the worktree instead of failing
    // on an existing branch.
    let run2 = fx.start(&fake, Some(repo.clone())).await;
    fx.worker("w2")
        .drive(run2)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        fx.data
            .path()
            .join("worktrees")
            .join(run2.to_string())
            .exists()
    );
}

async fn git_init_with_commit(repo: &Path) {
    std::fs::create_dir_all(repo).unwrap_or_else(|e| panic!("{e}"));
    git(repo, &["init", "-q", "-b", "main"]).await;
    std::fs::write(repo.join("README.md"), "first line of readme\n")
        .unwrap_or_else(|e| panic!("{e}"));
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
