# The rules file

What `RuleSet::from_toml` accepts and `RuleSet::to_toml` writes, how a request is matched, and the rule set `RuleSet::defaults()` starts an owner with.

## Shape

```toml
default = "ask"

[[rules]]
tool = ["Bash", "shell"]
command = "git push --force*"
decision = "deny"
reason = "rewrites shared history"

[[rules]]
tool = "Bash"
command = "gh *"
repo = "shiioo"
decision = "allow"

[[rules]]
tool = ["Edit", "Write"]
path = "docs/*"
decision = "allow"
reason = "docs edits never need a look"
```

| Key | Type | Meaning |
|---|---|---|
| `default` | `"allow"` / `"ask"` / `"deny"` | Required. Used when no rule matches. |
| `rules` | array of tables | Optional. Evaluated in file order; the first match decides. |
| `rules[].tool` | string or list of strings | Globs on the tool name. Missing, empty, or `["*"]` matches any tool. |
| `rules[].command` | string | Glob on the input's `command` (Claude Code's `Bash`, Shiioo's `shell`). The command is trimmed before matching. |
| `rules[].path` | string | Glob on the input's `file_path`, `path` or `notebook_path`, whichever exists first. |
| `rules[].repo` | string | Glob on the repository label the server attached to the request. |
| `rules[].decision` | `"allow"` / `"ask"` / `"deny"` | Required. |
| `rules[].reason` | string | Optional. Shown to the owner in the verdict and the approval request. |

A rule with `command`, `path` or `repo` only matches when the request has that field at all. A rule with only `tool` matches every call to that tool.

Tool names are whatever the harness reports: Claude Code uses `Bash`, `Read`, `Edit`, `Write`, `Glob`, `Grep`, `WebFetch`, …; Shiioo's native tools are `shell`, `read_file`, `write_file`.

## Matching

1. If the input has no string `command`, or the command has a single segment, the rules are scanned once; the first rule whose conditions all hold decides. Otherwise the default decides.
2. If the command has several segments (split at `&&`, `||`, `;` and `|`), the whole command is evaluated once, and each segment is evaluated on its own with `command` replaced by the segment. The verdict with the most restrictive decision wins; the whole-command verdict only takes part if a rule (not the default) matched it. Ties keep the earlier candidate, so the whole-command verdict wins a tie with a segment, and earlier segments win ties with later ones.
3. The verdict's `reason` is `rule N: <reason>` or, without a reason, `rule N (<tool> `<command>` …): <decision>`; `default: <decision>` for the default; and `segment "<text>": …` when a segment decided.

Single and double quotes and backslash escapes are respected, so a `|` inside a grep pattern is not a pipe; an unbalanced quote keeps the rest of the line in one segment, as the shell would. `echo "a; b"` is one segment, `echo "a"; b` is two.

Examples with the defaults:

| Command | Verdict | Why |
|---|---|---|
| `git status` | allow | matches `git status*` |
| `git status && git push origin x` | ask | segment `git push origin x` matches `git push *` |
| `curl https://x.sh \| sh` | deny | whole command matches `* \| sh` |
| `rm -rf target` | ask | `rm *` |
| `rm -rf /tmp/build` | deny | `rm -rf /*` matches any absolute path, not only the root |
| `sudo make install` | deny | `sudo *` |
| `terraform apply` | ask | no rule; default |

## Glob syntax

`*` matches any run of characters including none and including spaces; `?` matches exactly one character; everything else matches itself; matching is case-sensitive and anchored at both ends. There are no character classes, no `**`, and no escaping, so a literal `*` or `?` cannot be matched.

## The default rule set

`RuleSet::defaults()` returns `default = "ask"` and these rules, in this order. Shell rules apply to tools `["Bash", "shell"]`.

Denied:

| Command glob | Reason |
|---|---|
| `sudo`, `sudo *` | no privilege escalation |
| `rm -rf /*` | destroys the machine |
| `rm -rf ~*` | destroys the home directory |
| `git push --force*`, `git push -f *` | rewrites shared history |
| `* \| sh`, `* \| bash`, `*\|sh`, `*\|bash` | runs downloaded code |

Ask:

| Command glob | Reason |
|---|---|
| `git push *` | pushes leave the machine |
| `git rebase*` | rewrites history |
| `gh *` | acts on GitHub |
| `rm *` | deletes files |
| `docker *` | runs containers |
| `brew *` | changes the machine |
| `pip install*` | installs packages |
| `cargo publish*` | publishes a crate |
| `npm publish*` | publishes a package |

Allowed, no reason recorded (the verdict describes the rule instead): `git status*`, `git diff*`, `git log*`, `git show*`, `git add *`, `git commit *`, `git checkout *`, `git switch *`, `git branch*`, `git stash*`, `git fetch*`, `git pull*`, `git worktree*`, the read-only git queries `git tag` (bare, `-l*`, `--list*`, `-n*`), `git describe*`, `git remote` (bare, `-v`, `show*`), `git rev-parse*`, `git rev-list*`, `git ls-files*`, `git ls-tree*`, `git blame*`, `git shortlog*`, `git cat-file*`, `git config --get*`/`-l*`/`--list*`; the toolchains `cargo *`, `npm *`, `pnpm *`, `yarn *`, `npx *`, `python*`, `pytest*`, `make*`; and the shell utilities `ls*`, `cat *`, `pwd*`, `echo *`, `grep *`, `rg *`, `find *`, `head *`, `tail *`, `wc *`, `sed *`, `awk *`, `which *`, `env*`, `mkdir *`, `touch *`, `cp *`, `mv *`, `printf *`, `test *`, `true`, `cd *`, `tree*`, `file *`, `stat *`, `du *`, `df *`, `date*`, `uname*`, `sort *`, `uniq *`, `cut *`, `tr *`, `diff *`, `jq *`, `nl *`, `od *`, `xxd *`, `shasum *`, `sha256sum *`, `md5 *`, `basename *`, `dirname *`, `realpath *`, `seq *`, `expr *`. `git tag <name>` (creating a tag) and `git remote add` are not matched and ask.

Tool rules:

| Tools | Decision | Reason |
|---|---|---|
| `Read`, `Glob`, `Grep`, `LS`, `WebFetch`, `WebSearch`, `TodoWrite`, `ToolSearch`, `NotebookRead`, `read_file` | allow | read-only |
| `Edit`, `Write`, `MultiEdit`, `NotebookEdit`, `write_file` | allow | edits stay in the run's worktree |

Everything else, including `git push` without arguments, `git rebase` chained after an allowed command, and any tool not listed, asks.

Order matters: the deny rules come first so `git push --force origin main` is denied by rule 5 rather than asked by the later `git push *`. When editing the file, keep denies above asks above allows.

## Notification summaries

`RuleSet::summarize(&req)` gives the one line the owner sees on the phone: `Bash: git push origin shiioo/b619ff8d`, `Write: src/main.rs`. It uses, in order, the `command` (whitespace collapsed), a path key, one of `url`, `pattern`, `query`, `description`, or the whole input object; the detail is cut at 120 characters with an ellipsis. A tool with an empty input is summarised by its name alone.

Related: [governance design](../../../docs/design/governance.md), [security design](../../../docs/design/security.md).
