//! The owner's messages to a harness run: one that arrives while Claude
//! Code works is delivered by resuming its session when it finishes, and
//! a harness run that parks is woken by the next one, and a follow-up on
//! a finished run resumes its session in its worktree.

use shiioo_core::{EventStore, ManualClock, Payload, RunId, Seq, verify_chain};
use shiioo_runtime::{
    AgentSpec, HarnessSpec, LocalMachine, Origin, RunOutcome, RunSpec, Runtime, Toolbox,
    UnconfiguredProvider, Worker,
};
use shiioo_store::MemoryStore;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/claude-code-read.jsonl"
);
const SESSION: &str = "a11a080d-e85c-457e-9b31-578ebfba0c1c";
const ANSWER: &str = "first line of readme";

/// A `claude` that records each invocation's arguments and working
/// directory (the last line), replays the
/// recorded session (holding back its result line until a `go` file
/// exists), and replays a short completion when resumed.
struct Fake {
    dir: tempfile::TempDir,
}

impl Fake {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let fixture = std::fs::read_to_string(FIXTURE).unwrap_or_else(|e| panic!("{e}"));
        let lines: Vec<&str> = fixture.lines().collect();
        let write = |name: &str, body: String| {
            std::fs::write(dir.path().join(name), body).unwrap_or_else(|e| panic!("{e}"));
        };
        write("work.jsonl", lines[..5].join("\n") + "\n");
        write("result.jsonl", lines[5].to_owned() + "\n");
        write(
            "resume.jsonl",
            [lines[0], lines[4], lines[5]].join("\n") + "\n",
        );
        let d = dir.path().display();
        let script = format!(
            "#!/bin/sh\n\
             {{ printf '%s\\n' \"$@\"; pwd; echo ----; }} >> '{d}/args.txt'\n\
             case \" $* \" in *' --resume '*) cat '{d}/resume.jsonl'; exit 0;; esac\n\
             cat '{d}/work.jsonl'\n\
             n=0; while [ ! -e '{d}/go' ] && [ $n -lt 200 ]; do sleep 0.05; n=$((n+1)); done\n\
             cat '{d}/result.jsonl'\n"
        );
        let bin = dir.path().join("claude");
        std::fs::write(&bin, script).unwrap_or_else(|e| panic!("{e}"));
        Self { dir }
    }

    fn go(&self) {
        std::fs::write(self.dir.path().join("go"), "").unwrap_or_else(|e| panic!("{e}"));
    }

    /// Each invocation's arguments.
    fn invocations(&self) -> Vec<Vec<String>> {
        std::fs::read_to_string(self.dir.path().join("args.txt"))
            .unwrap_or_default()
            .split("----\n")
            .filter(|s| !s.is_empty())
            .map(|s| s.lines().map(str::to_owned).collect())
            .collect()
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }
}

/// The prompt an invocation was given: the argument after `--print`.
fn prompt(args: &[String]) -> &str {
    args.iter()
        .position(|a| a == "--print")
        .and_then(|i| args.get(i + 1))
        .map_or("", String::as_str)
}

struct Fixture {
    store: Arc<MemoryStore<ManualClock>>,
    machine: Arc<LocalMachine>,
    data: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let data = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        std::fs::create_dir_all(data.path().join("workspace")).unwrap_or_else(|e| panic!("{e}"));
        let machine = LocalMachine::in_memory(data.path().join("workspace"), Toolbox::default())
            .with_worktrees(data.path().join("worktrees"));
        Self {
            store: Arc::new(MemoryStore::new(ManualClock::at(1_700_000_000_000))),
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

    async fn start(&self, fake: &Fake, park_on_answer: bool) -> RunId {
        self.create(
            fake,
            None,
            "Read README.md and report its first line.",
            park_on_answer,
            None,
        )
        .await
    }

    async fn create(
        &self,
        fake: &Fake,
        repo: Option<PathBuf>,
        task: &str,
        park_on_answer: bool,
        origin: Option<Origin>,
    ) -> RunId {
        let mut harness = HarnessSpec::claude_code();
        harness.binary = Some(PathBuf::from("/bin/sh"));
        harness.binary_args = vec![fake.path().join("claude").to_string_lossy().into_owned()];
        harness.repo = repo;
        self.worker("creator")
            .create(&RunSpec {
                origin,
                park_on_answer,
                task: task.into(),
                agent: AgentSpec {
                    model: String::new(),
                    system: String::new(),
                    tools: vec!["Read".into()],
                    max_turns: 10,
                },
                runtime: Runtime::Harness(harness),
                machine: None,
            })
            .await
            .unwrap_or_else(|e| panic!("{e}"))
    }

    async fn say(&self, run: RunId, text: &str) {
        self.store
            .append_external(
                run,
                vec![Payload::UserMessage {
                    author: "owner".into(),
                    content: text.into(),
                }],
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
    }

    async fn log(&self, run: RunId) -> Vec<Payload> {
        let log = self
            .store
            .read(run, Seq(0))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(verify_chain(&log), Ok(()));
        log.iter().map(|e| (*e.payload).clone()).collect()
    }
}

fn count(log: &[Payload], f: impl Fn(&Payload) -> bool) -> usize {
    log.iter().filter(|p| f(p)).count()
}

#[tokio::test]
async fn a_message_sent_while_the_harness_works_is_the_next_resume_prompt() {
    let fake = Fake::new();
    let fx = Arc::new(Fixture::new());
    let run = fx.start(&fake, false).await;

    let driving = {
        let fx = Arc::clone(&fx);
        tokio::spawn(async move { fx.worker("w1").drive(run).await })
    };
    // Wait until the session is under way, then write to it.
    for _ in 0..200 {
        let log = fx.log(run).await;
        if count(&log, |p| matches!(p, Payload::ModelResponseRecorded { .. })) >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    fx.say(run, "Also count the lines.").await;
    fake.go();

    let outcome = driving
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(outcome, RunOutcome::Completed(ANSWER.into()));

    // Two processes: the task, then the session resumed with the message.
    let calls = fake.invocations();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert_eq!(
        prompt(&calls[0]),
        "Read README.md and report its first line."
    );
    assert!(calls[1].iter().any(|a| a == "--resume"), "{:?}", calls[1]);
    assert!(calls[1].iter().any(|a| a == SESSION), "{:?}", calls[1]);
    assert_eq!(prompt(&calls[1]), "Also count the lines.");

    // The first finish was not recorded as the end; the second was.
    let log = fx.log(run).await;
    assert_eq!(
        count(&log, |p| matches!(p, Payload::RunCompleted { .. })),
        1
    );
    assert_eq!(
        count(&log, |p| matches!(p, Payload::HarnessResult { .. })),
        2
    );
    assert_eq!(
        count(&log, |p| matches!(p, Payload::SandboxStarted { .. })),
        2
    );
    assert!(matches!(log.last(), Some(Payload::RunCompleted { .. })));
}

#[tokio::test]
async fn a_parked_harness_run_is_resumed_with_the_message_that_wakes_it() {
    let fake = Fake::new();
    fake.go();
    let fx = Fixture::new();
    let run = fx.start(&fake, true).await;

    let first = fx
        .worker("w1")
        .drive(run)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(first, RunOutcome::Parked("input".into()));

    fx.say(run, "Now the second line, please.").await;
    let second = fx
        .worker("w2")
        .drive(run)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(second, RunOutcome::Parked("input".into()));

    let calls = fake.invocations();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert_eq!(prompt(&calls[1]), "Now the second line, please.");
    let log = fx.log(run).await;
    assert_eq!(count(&log, |p| matches!(p, Payload::RunWoken { .. })), 1);
    assert_eq!(count(&log, |p| matches!(p, Payload::RunParked { .. })), 2);
    assert_eq!(
        count(&log, |p| matches!(p, Payload::RunCompleted { .. })),
        0
    );
}

#[tokio::test]
async fn a_follow_up_resumes_the_finished_runs_session_in_its_worktree() {
    let fake = Fake::new();
    fake.go();
    let fx = Fixture::new();
    let repo = fx.data.path().join("repo");
    git_init_with_commit(&repo).await;

    let first = fx
        .create(
            &fake,
            Some(repo.clone()),
            "Read README.md and report its first line.",
            false,
            None,
        )
        .await;
    let outcome = fx
        .worker("w1")
        .drive(first)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(outcome, RunOutcome::Completed(ANSWER.into()));

    let end = fx.log(first).await.len() as u64 - 1;
    let follow = fx
        .create(
            &fake,
            Some(repo),
            "Now count its lines.",
            false,
            Some(Origin {
                run: first,
                seq: Seq(end),
                workspace: first,
                session: Some(SESSION.into()),
            }),
        )
        .await;
    let outcome = fx
        .worker("w2")
        .drive(follow)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(outcome, RunOutcome::Completed(ANSWER.into()));

    let calls = fake.invocations();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert!(!calls[0].iter().any(|a| a == "--resume"), "{:?}", calls[0]);
    assert!(calls[1].iter().any(|a| a == SESSION), "{:?}", calls[1]);
    assert_eq!(prompt(&calls[1]), "Now count its lines.");

    // Both processes ran in the first run's worktree; the follow-up has
    // none of its own.
    let worktrees = fx.data.path().join("worktrees");
    let cwd = |c: &Vec<String>| {
        c.last()
            .and_then(|p| std::fs::canonicalize(p).ok())
            .unwrap_or_default()
    };
    let expected = std::fs::canonicalize(worktrees.join(first.to_string())).ok();
    assert_eq!(Some(cwd(&calls[0])), expected);
    assert_eq!(Some(cwd(&calls[1])), expected);
    assert!(!worktrees.join(follow.to_string()).exists());
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

async fn git(repo: &Path, args: &[&str]) {
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
}
