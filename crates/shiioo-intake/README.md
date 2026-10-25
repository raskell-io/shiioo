# shiioo-intake

Work intake from GitHub: an issue the owner labels `shiioo` becomes a run. The intake polls each configured repository with the `gh` CLI, starts a run for every new labelled issue, comments with the run id and branch, and when the run ends comments with the outcome and swaps the label to `shiioo:done` or `shiioo:failed`. A small JSON state file remembers which issues it has already handled, so a restart neither starts a second run nor forgets to report on one.

## Where it sits

- Depends on: `serde`, `serde_json`, `thiserror`, `tokio` (`process`, `time`, `fs`, `sync`). It does not depend on `shiioo-core`: runs are started through a trait the server implements.
- Used by: the `shiioo` binary, which implements `IntakeSink` on the office and spawns `run_intake`.
- Talks to GitHub only through `gh`, run with the repository as its working directory, so per-repository account wrappers and `gh auth` state apply unchanged.

## Public API

| Item | What it is |
|---|---|
| `RepoIntake { label, path }` | A watched repository: its label in Shiioo's config and the checkout `gh` runs in. |
| `IntakeConfig { repos, issue_label, interval, state_file, gh_bin, gh_prefix_args }` | `new(repos, state_file)` gives label `shiioo`, interval 60 s, `gh_bin` from `SHIIOO_GH_BIN` or the path, no prefix args. |
| `Issue { repo, number, title, body, url }` | What the intake hands to `task_for`. |
| `Started { run_id, branch }` | What the sink returns when a run is started. |
| `IntakeSink` | `start_run(repo_label, task) -> Result<Started, String>`; `run_outcome(run_id) -> Option<Result<summary, reason>>`. Implemented by the server. |
| `run_intake(config, sink)` | Polls forever until the future is dropped. Never panics; errors are logged to stderr and retried next poll. |
| `task_for(&issue, branch)`, `BRANCH_PLACEHOLDER`, `with_branch(task, branch)` | The run's prompt. The intake passes `{branch}` because the branch is derived from the run id; the sink substitutes it. |
| `Gh`, `GhError`, `GhIssue` | The `gh` wrapper: `new(binary)`, `with_prefix(binary, prefix)`, `list_issues`, `comment`, `relabel`, `ensure_labels`. |
| `State`, `IssueRecord`, `IssueStatus` | The state file: `key(repo, number)`, `load`, `save`; records of `Running`, `Done`, `Failed`. |
| `BoxFuture<'a, T>` | The pinned boxed future the sink trait returns. |

## Using it

```rust
use shiioo_intake::{IntakeConfig, RepoIntake, run_intake};
use std::sync::Arc;

let config = IntakeConfig::new(
    vec![RepoIntake { label: "demo".into(), path: "/srv/repos/demo".into() }],
    "/srv/shiioo/intake-state.json",
);
tokio::spawn(run_intake(config, Arc::clone(&office) as Arc<dyn shiioo_intake::IntakeSink>));
```

The sink's `start_run` receives a task containing `{branch}`; call `with_branch(&task, &branch)` once the run id is known, as the test sink in `tests/intake.rs` does.

## Testing

```sh
cargo test -p shiioo-intake
```

The integration tests write a `gh` stand-in script that logs every invocation and answers `issue list` with two open issues. It runs through `/bin/sh <script>` via `gh_prefix_args`, because macOS stalls the first direct execution of a freshly written script. Cases: new issues become runs and outcomes are reported with the right comments and labels; a corrupt state file stops the intake instead of re-running everything; the `Gh` wrapper parses the fake's output. Unit tests check the task text and the state key format.

| Variable | Use |
|---|---|
| `SHIIOO_GH_BIN` | The `gh` executable when `IntakeConfig::gh_bin` is `None`. Defaults to `gh` on the path. |

## Invariants

- An issue is identified by `<repo label>#<number>`. Once in the state file it is never started again, even if the label is removed and re-added. Delete the record to re-run an issue.
- The state is saved before the comment that announces a run, so a crash between the two cannot cause a duplicate run; it can lose the comment.
- A corrupt state file is an error, not an empty state. The intake logs and stays off until the file is fixed or removed.
- `gh` is run in the repository directory with the label protocol's three labels ensured at startup; label creation failures are logged and ignored.
- Comments are cut at 2 000 characters. At most 50 open labelled issues are fetched per poll.

Step by step: [`docs/flow.md`](docs/flow.md).

## Further reading

- [Intake design](../../docs/design/intake.md), [architecture overview](../../docs/architecture.md), [deployment design](../../docs/design/deployment.md) for where the state file and repositories live.
