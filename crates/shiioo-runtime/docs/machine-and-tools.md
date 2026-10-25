# Machine, tools and worktrees

The machine is where a native run's tool calls actually execute. It exists
so that a worker crash between "the model asked for this call" and "the
result is in the log" never runs a side effect twice without anyone knowing.
`src/machine.rs` holds the machine, `src/tools.rs` the tools it runs,
`src/workspace.rs` the git worktrees harness runs work in.

## LocalMachine

```rust
LocalMachine::in_memory(workspace, toolbox)              // tests
LocalMachine::open(workspace, journal_path, toolbox)     // production
    .with_worktrees(root)                                // allow repo runs
```

- `workspace()`: the directory tools run in.
- `toolbox()`: the tools this machine can execute.
- `worktrees_root()`: where per-run worktrees go; `None` means repo runs
  fail with `this machine has no worktree directory for repo runs`.
- `status(run_id, epoch, call_id) -> CallStatus`: what the journal knows
  about a call: `NotReceived`, `Started`, or `Finished(output)`.
- `execute(Invocation) -> ToolOutput`: run a call exactly once.
- `rerun(Invocation)`: reset a `Started` entry, then `execute`.

An `Invocation` is `{ run_id, epoch, call_id, tool, input }`.

### Fencing

Every call to `status`, `execute` or `rerun` presents the worker's lease
epoch. The machine keeps the highest epoch it has seen per run in a `fence`
journal record. A lower epoch is refused with `MachineError::Fenced {
presented, current }`, which the worker turns into `RunOutcome::LostLease`.
So even a worker that still believes it owns a run cannot execute anything on
the machine once a successor has touched it.

The fence is checked once at the start of `execute`; the `finished` record
is written without re-checking, so a call that started under a valid epoch
always gets its outcome journaled.

### The journal

One JSON object per line, `kind`-tagged, `sync_data` after every write:

```json
{"kind":"fence","run":"<run id>","epoch":3}
{"kind":"started","run":"<run id>","call":"<call id>"}
{"kind":"finished","run":"<run id>","call":"<call id>","output":{"content":"…","is_error":false}}
{"kind":"reset","run":"<run id>","call":"<call id>"}
```

`open` replays the file into memory and ignores a torn last line (a crash
mid-write). `execute` does:

1. fence check;
2. `Finished` in journal → return the journaled output, do not run;
3. `Started` in journal → `MachineError::OutcomeUnknown(call_id)`;
4. otherwise write `started`, run the tool, write `finished`.

Together with the worker's `recover()` (see
[worker-loop.md](worker-loop.md#recovery)) this gives: pure and idempotent
tools may be re-executed after a crash, side-effecting tools whose `started`
has no `finished` stop the run with `NeedsAttention`.

`crash_before_next_finish()` is a test-only hook that makes the next
`execute` die after `started` (`MachineError::SimulatedCrash`).

## Tools

```rust
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;                       // name, description, JSON schema
    fn class(&self, input: &Value) -> ToolClass;      // Pure | Idempotent { key } | SideEffecting
    fn execute(&self, input: &Value, workspace: &Path) -> BoxFuture<ToolOutput>;
}
```

`ToolOutput { content, is_error }` with `ToolOutput::ok(text)` and
`ToolOutput::error(text)`. Output longer than `OUTPUT_KEEP` (32 KiB) keeps
the head and tail with `… [N bytes omitted] …` in between.

`Toolbox::builtin()` holds three tools; `Toolbox::with(tool)` adds one,
`get(name)` looks one up, `specs(names)` returns the specs the model sees.

| Tool | Class | Behaviour |
| --- | --- | --- |
| `read_file` | `Pure` | Reads `path` relative to the workspace; refuses absolute paths and `..`. |
| `write_file` | `Idempotent { key: digest(path, content) }` | Writes `content` to `path`, creating parent directories. Same key, same effect, so a rerun is safe. |
| `shell` | `SideEffecting` | `sh -c <command>` in the workspace, 120 s timeout. Output is `exit code: N` followed by `stdout:` and `stderr:` sections; `is_error` on non-zero exit. |

The idempotency key is a length-prefixed SHA-256 over the parts
(`digest_hex`, crate-private), so `("a", "bc")` and `("ab", "c")` differ.

Which tools a run may use is decided twice: `AgentSpec.tools` names the
tools the agent is allowed, and the machine's toolbox names the tools that
exist. A call to anything else gets the error output
`tool '<name>' is not available to this agent` and the run continues.

Harness runs do not use the toolbox at all; the harness brings its own tools
and the adapter only records them (with a class, so recovery semantics are
the same in the log).

## Worktrees

Harness runs on a repository work in their own git worktree so several runs
can share one checkout without stepping on each other.

- `branch_for(run_id)` → `shiioo/<first 8 chars of the run id>`.
- `ensure(root, repo, run_id) -> Worktree { path, branch }`:
  - path is `<root>/<run id>`; if it already has a `.git` entry the worktree
    is reused (a resumed run keeps its files);
  - otherwise creates `root`, runs `git worktree prune`, checks `git branch
    --list <branch>`, then `git worktree add -b <branch> <path>` for a new
    branch or `git worktree add <path> <branch>` when the branch already
    exists.
- `remove(repo, worktree)` runs `git worktree remove --force`; the branch is
  kept so the owner can inspect and merge it.

Git failures surface as `WorktreeError::Git { args, stderr }`.

The API's `GET /v1/runs/{id}/diff` (and the dashboard's Diff button) rely on this
layout: the diff is `git diff --stat -p HEAD...shiioo/<run>` in the
repository.

See also: [tools and machines design](../../../docs/design/tools-and-machines.md),
[execution design](../../../docs/design/execution.md).
