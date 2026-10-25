# Intake flow

What `run_intake` does on each poll, what it writes to GitHub and to disk, and how failures are handled.

## Label protocol

| Label | Colour | Meaning |
|---|---|---|
| `<label>` (default `shiioo`) | `1D76DB` | The owner wants an agent to work on this issue. |
| `<label>:done` | `0E8A16` | The run finished well. |
| `<label>:failed` | `B60205` | The run failed, was cancelled, or stopped for the owner. |

`Gh::ensure_labels` creates or updates all three at startup for each repository. Failures are logged and ignored, because the labels may exist already or the token may lack the permission, and neither stops the intake.

## Startup

1. Build the `Gh` wrapper from `gh_bin` and `gh_prefix_args`.
2. `State::load(state_file)`. A missing file is an empty state. A file that exists but does not parse is an error: the intake logs `cannot read …; the intake stays off until it is fixed or removed` and returns. Starting over silently would re-run every open labelled issue.
3. Log the repositories, label and interval; ensure labels.

## Each poll

For every repository, `poll_repo`:

1. `gh issue list --label <label> --state open --json number,title,body,url,labels --limit 50`, run with the repository as the working directory.
2. For each issue whose key `<repo label>#<number>` is not in the state:
   - Build the task with `task_for(&issue, "{branch}")`.
   - `sink.start_run(repo_label, task)`. On `Err`, log and move on; the issue is retried next poll.
   - Record `IssueRecord { run_id, status: Running, started_at }` and save the state file.
   - Comment `🏃 Shiioo started run `<run id>` on branch `<branch>`.` A failed comment is logged; the record stays.

Then `report_outcomes`, for every record with status `Running`:

1. `sink.run_outcome(run_id)`. `None` means still running; nothing happens.
2. `Some(Ok(summary))`: status `Done`, comment `✅ Done: <summary>`, relabel `<label>` to `<label>:done`.
3. `Some(Err(reason))`: status `Failed`, comment `❌ <reason>`, relabel to `<label>:failed`.
4. The state is saved before the comment and relabel; summaries and reasons are cut at 2 000 characters.

Then sleep `interval` (default 60 s) and repeat.

Issues are processed one repository at a time; the sink calls are awaited in order.

## The task

`task_for` produces the whole prompt the run gets:

```
Work on GitHub issue #<number>: <title>
<url>

Issue description:
---
<body, or "(no description)">
---

You are working in a git worktree on branch `<branch>`. Make the change, commit with clear messages,
push the branch with `git push -u origin <branch>`, then open a pull request with
`gh pr create --fill --head <branch>` whose description says `Closes #<number>`. If something is
unclear or the issue asks for something you shouldn't do, stop and explain instead of guessing.
```

The intake passes `BRANCH_PLACEHOLDER` (`{branch}`) as the branch because the branch name is derived from the run id, which does not exist yet. The sink substitutes it with `with_branch` once the run is created. Whether the push and the pull request actually happen is governed by the owner's rules: `git push *` and `gh *` ask by default.

## The state file

`<data>/intake-state.json`, pretty-printed JSON written to `intake-state.json.tmp` and renamed into place:

```json
{
  "issues": {
    "demo#2": {
      "run_id": "018f…",
      "status": "running",
      "started_at": 1761300000000
    }
  }
}
```

`status` is `running`, `done` or `failed`. Keys are `<repo label>#<issue number>`, kept in a `BTreeMap` so the file is stable to diff. Records are never removed by the intake.

## `gh` invocations

| Purpose | Command |
|---|---|
| list | `gh issue list --label <label> --state open --json number,title,body,url,labels --limit 50` |
| comment | `gh issue comment <number> --body <text>` |
| relabel | `gh issue edit <number> --remove-label <label> --add-label <label>:done` (or `:failed`) |
| labels | `gh label create <name> --color <hex> --description <text> --force`, once per label |

Every call runs with the repository path as the working directory and `gh_prefix_args` in front (empty in production, `["/bin/sh", "<script>"]`-style in tests). Non-zero exit, unparsable JSON and spawn failures surface as `GhError`.

## Failure handling

| Failure | Effect |
|---|---|
| `gh issue list` fails | Logged; the repository is skipped this poll. |
| `start_run` returns `Err` | Logged; the issue is retried every poll until it succeeds or the label is removed. |
| State save fails | Logged; the intake continues with its in-memory state. A restart may then start a second run for an issue it already started. |
| Comment or relabel fails | Logged; the state already says `Done`/`Failed`, so the outcome is not reported again. |
| Process restarts mid-run | The record is `Running`; `run_outcome` is asked again on the next poll. |
| Issue closed or unlabelled while running | Nothing changes; the outcome is still commented and relabelled when the run ends. |

## Not covered today

- Re-running an issue after `done` or `failed` without editing the state file.
- Pull-request or comment-driven intake, issue assignment, or multiple labels per repository *(planned, see the intake design)*.
- Pagination beyond 50 open labelled issues per repository.

Related: [intake design](../../../docs/design/intake.md), [governance design](../../../docs/design/governance.md) for the rules the run's pushes and `gh` calls go through.
