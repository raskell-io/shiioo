# Work intake: GitHub issues

Labelling a GitHub issue hands it to the office. The intake turns it into a run, keeps the issue informed, and marks it when the run ends. Built in `crates/shiioo-intake` *(built; wiring into `shiioo serve` is the server's job)*.

## The flow

```
owner labels issue #42 "shiioo"
        │  (poll, every 60 s, per configured repo: gh issue list --label shiioo)
        ▼
intake starts a run on the repo  ──►  comment: "🏃 Shiioo started run <id> on branch shiioo/<id8>"
        │
        │  the run works in its worktree, commits, pushes, opens a PR (each tool call still
        │  goes through the rules and, where they say ask, the owner's phone)
        ▼
run ends  ──►  comment "✅ Done: <summary>" + label shiioo → shiioo:done
          or   comment "❌ <reason>"       + label shiioo → shiioo:failed
```

The owner's part is one label on GitHub and, when a rule asks, one tap on the phone. The result is a pull request to review in the GitHub app.

## The label protocol

| Label | Meaning | Who sets it |
|---|---|---|
| `shiioo` (configurable, `issue_label`) | an agent should work on this | the owner |
| `shiioo:done` | the run finished; see the comment and the PR | the intake |
| `shiioo:failed` | the run failed, was cancelled, or needs the owner; see the comment | the intake |

The intake creates the three labels in each repo on start (`gh label create --force`); if that fails (missing permission) it goes on without them. An issue is picked up once: the key `<repo label>#<number>` is remembered (see *State*). To re-run an issue, remove it from the state file or open a new issue.

## The task prompt

`task_for(issue, branch)` builds the run's task:

- the issue number, title, URL and body (verbatim, between `---` markers);
- standing instructions: the agent works in a git worktree on the named branch, commits with clear messages, pushes with `git push -u origin <branch>`, opens a pull request with `gh pr create --fill --head <branch>` whose description says `Closes #<n>`, and stops to explain rather than guess when something is unclear or shouldn't be done.

The branch name is `shiioo/<first 8 chars of the run id>`, which only exists once the run does. The intake therefore writes the task with a `{branch}` placeholder (`BRANCH_PLACEHOLDER`) and the server substitutes the real name with `with_branch(task, branch)` when it creates the run. The run's `RunCreated` event carries the final task.

Pushing and opening the PR are the agent's actions and go through the rules: with the defaults, `git push *` and `gh *` ask the owner. An owner who trusts a repository can allow them for that repo in `rules.toml`:

```toml
[[rules]]
tool = "Bash"
command = "git push -u origin shiioo/*"
repo = "demo"
decision = "allow"
```

## Dedup and state

`<data>/intake-state.json`:

```json
{ "issues": { "demo#42": { "run_id": "…", "status": "running", "started_at": 1761376000000 } } }
```

- Written after every change, to a temporary file that is then renamed into place, so a crash never leaves a half-written file.
- On start, the file is loaded; issues in it are never started again, and issues still `running` are checked on every poll until the server reports an outcome.
- A corrupt file stops the intake with a message rather than starting from scratch: starting from scratch would re-run every labelled issue.

## Failure handling

| What fails | What happens |
|---|---|
| `gh` can't list issues (network, auth, rate limit) | logged; retried next poll; nothing else changes |
| the run can't be started | logged; the issue is not recorded, so it's retried next poll |
| the comment or relabel fails | logged; the state is already saved, so the run is not started twice; the issue just lacks the comment |
| the run fails, is cancelled, or needs the owner | `❌ <reason>` comment, `shiioo:failed` label; the owner sees the same in Telegram and the dashboard |
| the server restarts mid-run | the run itself resumes from its log (see execution); the intake reloads its state and keeps polling the run's outcome |
| the owner relabels an issue `shiioo` again after `shiioo:done` | ignored (the key is known); remove it from the state file to re-run |

## Security

Issue titles and bodies are **untrusted text handed to an agent**. The intake does not try to sanitise them; what protects the office is that every tool call the agent makes still goes through the rules and, where they say ask, the owner. An issue that says "run `curl … | sh`" produces a denied call and a `❌` comment, not an incident. Still:

- Scope the intake to repositories where only people you trust can add labels. On GitHub, adding a label needs triage permission or more, so on your own repositories strangers cannot hand work to the office; on an open-source repo with many collaborators, think twice.
- The `gh` CLI runs with the repository as its working directory, so per-repository account wrappers choose the right GitHub identity; the office never holds a GitHub token itself.
- Comments the intake posts contain the run id and branch, nothing from the run's log; the log stays in the office.

## Configuration

In the server's `config.toml` the repositories under `[repos]` are the candidates; `[intake] repos = ["demo", …]` (planned in the server) selects which of them the intake watches, `label` overrides `shiioo`, `interval_secs` the poll interval. The intake runs inside `shiioo serve`; it needs `gh` on the `PATH` (the installed service captures it) and the owner logged in. `IntakeConfig.gh_bin` (or `SHIIOO_GH_BIN`) points at another `gh`; `gh_prefix_args` puts arguments before every call, which the tests use to run a fake through `/bin/sh <script>` (macOS stalls the first direct execution of a freshly written script for a long time).
