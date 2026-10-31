//! Taking over a run: the harness runs interactively under a terminal with
//! the run's hooks and environment, what happened is imported from its
//! session file, and the run is left parked. A headless worker gives way
//! to a takeover request, and a takeover the office never ended is closed
//! when the run is next driven.

use serde_json::json;
use shiioo_core::{EventStore, ManualClock, Payload, RunId, Seq, WorkerId, verify_chain};
use shiioo_runtime::harness::transcript::{Homes, project_dir};
use shiioo_runtime::{
    AgentSpec, HarnessSpec, HookCommand, HookEndpoint, LocalMachine, RunOutcome, RunSpec,
    RunStatus, Runtime, TakeoverControl, Toolbox, UnconfiguredProvider, Worker, run_status,
};
use shiioo_store::MemoryStore;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{oneshot, watch};

struct Fixture {
    store: Arc<MemoryStore<ManualClock>>,
    clock: ManualClock,
    machine: Arc<LocalMachine>,
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        std::fs::create_dir_all(dir.path().join("workspace")).unwrap_or_else(|e| panic!("{e}"));
        let clock = ManualClock::at(1_700_000_000_000);
        Self {
            store: Arc::new(MemoryStore::new(clock.clone())),
            clock,
            machine: Arc::new(LocalMachine::in_memory(
                dir.path().join("workspace"),
                Toolbox::default(),
            )),
            dir,
        }
    }

    fn worker(&self, id: &str) -> Worker<MemoryStore<ManualClock>> {
        Worker::new(
            Arc::clone(&self.store),
            Arc::new(UnconfiguredProvider),
            Arc::clone(&self.machine),
            id,
        )
        .with_hooks(HookEndpoint {
            server_url: "http://127.0.0.1:9".into(),
            token: "hook-token".into(),
            command: HookCommand::Curl,
        })
    }

    fn workspace(&self) -> PathBuf {
        self.dir.path().join("workspace")
    }

    fn homes(&self) -> Homes {
        Homes {
            claude: self.dir.path().join("claude-home"),
            codex: self.dir.path().join("codex-home"),
        }
    }

    /// Claude Code's session file for the workspace.
    fn session_file(&self, session: &str) -> PathBuf {
        self.homes()
            .claude
            .join("projects")
            .join(project_dir(&self.workspace()))
            .join(format!("{session}.jsonl"))
    }

    async fn create(&self, script: &Path) -> RunId {
        let mut harness = HarnessSpec::claude_code();
        harness.binary = Some(PathBuf::from("/bin/sh"));
        harness.binary_args = vec![script.to_string_lossy().into_owned()];
        harness.env.insert(
            "FAKE_SESSION_FILE".into(),
            self.session_file("s-take").to_string_lossy().into_owned(),
        );
        self.worker("creator")
            .create(&RunSpec {
                origin: None,
                park_on_answer: false,
                task: "Fix the flaky test.".into(),
                agent: AgentSpec {
                    model: String::new(),
                    system: String::new(),
                    tools: vec![],
                    max_turns: 10,
                },
                runtime: Runtime::Harness(harness),
                machine: None,
            })
            .await
            .unwrap_or_else(|e| panic!("{e}"))
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

    async fn status(&self, run: RunId) -> RunStatus {
        let log = self
            .store
            .read(run, Seq(0))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        run_status(&log)
    }
}

/// An interactive "harness": says what it was started with, echoes what
/// is typed, and on `hello` writes a turn into its session file.
fn fake(dir: &Path) -> PathBuf {
    let path = dir.join("interactive.sh");
    let user = json!({"type": "user", "uuid": "u1", "sessionId": "s-take",
        "message": {"role": "user", "content": "Use the existing backoff helper."}});
    let assistant = json!({"type": "assistant", "uuid": "a1", "sessionId": "s-take",
        "message": {"model": "m", "role": "assistant", "content": [{"type": "text", "text": "Switched to backoff."}]}});
    let script = format!(
        "#!/bin/sh\n\
         printf 'args:%s\\n' \"$*\"\n\
         printf 'run:%s\\n' \"$SHIIOO_RUN_ID\"\n\
         if [ -t 0 ]; then echo tty:yes; else echo tty:no; fi\n\
         while IFS= read -r line; do\n\
           if [ \"$line\" = hello ]; then\n\
             mkdir -p \"$(dirname \"$FAKE_SESSION_FILE\")\"\n\
             echo '{user}' >> \"$FAKE_SESSION_FILE\"\n\
             echo '{assistant}' >> \"$FAKE_SESSION_FILE\"\n\
           fi\n\
           echo \"got:$line\"\n\
         done\n"
    );
    std::fs::write(&path, script).unwrap_or_else(|e| panic!("{e}"));
    path
}

/// Reads terminal output until it contains `want`.
async fn read_until(
    rx: &mut tokio::sync::broadcast::Receiver<Arc<[u8]>>,
    seen: &mut String,
    want: &str,
) {
    let found = tokio::time::timeout(Duration::from_secs(10), async {
        while !seen.contains(want) {
            match rx.recv().await {
                Ok(chunk) => seen.push_str(&String::from_utf8_lossy(&chunk)),
                Err(_) => return,
            }
        }
    })
    .await;
    assert!(
        found.is_ok() && seen.contains(want),
        "no {want:?} in {seen:?}"
    );
}

#[tokio::test]
async fn the_owner_types_into_the_harness_and_the_session_is_imported() {
    let fx = Arc::new(Fixture::new());
    let script = fake(fx.dir.path());
    let run = fx.create(&script).await;

    let (ready_tx, ready) = oneshot::channel();
    let (end_tx, end) = watch::channel(None);
    let taking = {
        let fx = Arc::clone(&fx);
        let homes = fx.homes();
        tokio::spawn(async move {
            fx.worker("desk")
                .take_over(
                    run,
                    "owner",
                    TakeoverControl {
                        ready: ready_tx,
                        end,
                        size: (100, 30),
                        homes,
                    },
                )
                .await
        })
    };
    let terminal = tokio::time::timeout(Duration::from_secs(10), ready)
        .await
        .unwrap_or_else(|_| panic!("the terminal never came"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(fx.status(run).await, RunStatus::TakenOver { by } if by == "owner"));

    let (_, mut rx) = terminal.attach();
    let mut seen = String::new();
    terminal
        .write(b"hello\r")
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    read_until(&mut rx, &mut seen, "got:hello").await;
    // Started with the task (no session yet), on a terminal, with the hook
    // environment.
    let (back, _) = terminal.attach();
    let back = String::from_utf8_lossy(&back).into_owned();
    assert!(back.contains("args:Fix the flaky test."), "{back}");
    assert!(back.contains(&format!("run:{run}")), "{back}");
    assert!(back.contains("tty:yes"), "{back}");

    end_tx.send_replace(Some("handed_back".into()));
    let outcome = tokio::time::timeout(Duration::from_secs(15), taking)
        .await
        .unwrap_or_else(|_| panic!("the takeover did not end"))
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(outcome, RunOutcome::Parked("input".into()));
    assert!(terminal.has_exited());

    let log = fx.log(run).await;
    let at = |f: &dyn Fn(&Payload) -> bool| log.iter().position(f).unwrap_or(usize::MAX);
    let started = at(&|p| matches!(p, Payload::TakeoverStarted { by } if by == "owner"));
    let said = at(
        &|p| matches!(p, Payload::UserMessage { content, .. } if content == "Use the existing backoff helper."),
    );
    let replied = at(&|p| matches!(p, Payload::ModelResponseRecorded { .. }));
    let ended = at(&|p| {
        matches!(p, Payload::TakeoverEnded { reason, session }
        if reason == "handed_back" && session.as_deref() == Some("s-take"))
    });
    assert!(
        started < said && said < replied && replied < ended && ended < log.len(),
        "{log:#?}"
    );
    assert!(
        matches!(log.last(), Some(Payload::RunParked { waiting_for, .. }) if waiting_for == "input")
    );
    assert!(matches!(fx.status(run).await, RunStatus::Parked { .. }));
}

#[tokio::test]
async fn a_working_harness_gives_way_to_a_takeover() {
    let fx = Fixture::new();
    let slow = fx.dir.path().join("slow.sh");
    std::fs::write(&slow, "#!/bin/sh\nsleep 30\n").unwrap_or_else(|e| panic!("{e}"));
    let run = fx.create(&slow).await;
    let worker = fx.worker("w1");
    let driving = worker.drive(run);
    let request = async {
        // Wait for the process, then ask for the keyboard.
        for _ in 0..200 {
            if fx
                .log(run)
                .await
                .iter()
                .any(|p| matches!(p, Payload::SandboxStarted { .. }))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        fx.store
            .append_external(run, vec![Payload::TakeoverRequested { by: "owner".into() }])
            .await
            .unwrap_or_else(|e| panic!("{e}"));
    };
    let started = std::time::Instant::now();
    let (outcome, ()) = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(driving, request)
    })
    .await
    .unwrap_or_else(|_| panic!("the worker did not give way"));
    assert_eq!(
        outcome.unwrap_or_else(|e| panic!("{e}")),
        RunOutcome::Parked("takeover".into())
    );
    assert!(started.elapsed() < Duration::from_secs(14));
    // Nothing wakes it but the takeover itself.
    assert_eq!(
        fx.worker("w2")
            .drive(run)
            .await
            .unwrap_or_else(|e| panic!("{e}")),
        RunOutcome::Parked("takeover".into())
    );
}

#[tokio::test]
async fn a_takeover_the_office_never_ended_is_closed_and_the_run_waits() {
    let fx = Fixture::new();
    let slow = fx.dir.path().join("slow.sh");
    std::fs::write(&slow, "#!/bin/sh\nsleep 30\n").unwrap_or_else(|e| panic!("{e}"));
    let run = fx.create(&slow).await;
    // An office that started a takeover and died: the lease runs out.
    let lease = fx
        .store
        .acquire_lease(run, WorkerId("old-office".into()), 1_000)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    fx.store
        .append(
            &lease,
            Seq(2),
            vec![Payload::TakeoverStarted { by: "owner".into() }],
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    fx.clock.advance_ms(5_000);

    let outcome = fx
        .worker("after-restart")
        .drive(run)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(outcome, RunOutcome::Parked("input".into()));
    let log = fx.log(run).await;
    assert!(log.iter().any(
        |p| matches!(p, Payload::TakeoverEnded { reason, .. } if reason == "office_restarted")
    ));
    assert!(matches!(fx.status(run).await, RunStatus::Parked { .. }));
    // Nothing ran headless.
    assert!(
        !log.iter()
            .any(|p| matches!(p, Payload::SandboxStarted { .. }))
    );
}

#[tokio::test]
async fn a_takeover_waits_for_the_same_process_worker_to_park() {
    // The office's workers and its takeovers share an id prefix; a holder
    // may take its lease again, so the takeover must not.
    let fx = Arc::new(Fixture::new());
    let script = fake(fx.dir.path());
    let slow = fx.dir.path().join("slow.sh");
    std::fs::write(&slow, format!("#!/bin/sh\ncase \" $* \" in *' --print '*) sleep 30;; *) exec /bin/sh '{}' \"$@\";; esac\n", script.display()))
        .unwrap_or_else(|e| panic!("{e}"));
    let run = fx.create(&slow).await;
    let driving = {
        let fx = Arc::clone(&fx);
        tokio::spawn(async move { fx.worker("office").drive(run).await })
    };
    for _ in 0..200 {
        if fx
            .log(run)
            .await
            .iter()
            .any(|p| matches!(p, Payload::SandboxStarted { .. }))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    fx.store
        .append_external(run, vec![Payload::TakeoverRequested { by: "owner".into() }])
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let (ready_tx, ready) = oneshot::channel();
    let (end_tx, end) = watch::channel(None);
    let taking = {
        let fx = Arc::clone(&fx);
        let homes = fx.homes();
        tokio::spawn(async move {
            fx.worker("office")
                .take_over(
                    run,
                    "owner",
                    TakeoverControl {
                        ready: ready_tx,
                        end,
                        size: (80, 24),
                        homes,
                    },
                )
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(20), ready)
        .await
        .unwrap_or_else(|_| panic!("the takeover never started"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        driving
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|e| panic!("{e}")),
        RunOutcome::Parked("takeover".into())
    );
    end_tx.send_replace(Some("handed_back".into()));
    taking
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|e| panic!("{e}"));
    let log = fx.log(run).await;
    let parked = log.iter().position(
        |p| matches!(p, Payload::RunParked { waiting_for, .. } if waiting_for == "takeover"),
    );
    let started = log
        .iter()
        .position(|p| matches!(p, Payload::TakeoverStarted { .. }));
    assert!(parked < started && started.is_some(), "{log:#?}");
    assert!(!log.iter().any(
        |p| matches!(p, Payload::TakeoverEnded { reason, .. } if reason == "office_restarted")
    ));
}
