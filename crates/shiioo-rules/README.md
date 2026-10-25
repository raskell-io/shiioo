# shiioo-rules

The owner's allow / ask / deny rules for what agents may do. One TOML file, one `RuleSet`, one question: given a tool call (tool name, JSON input, optional repository label), is it allowed, does it need the owner's approval, or is it denied? First matching rule wins; otherwise the default. Chained shell commands are judged segment by segment as well as whole, and the most restrictive answer wins.

This is the single-owner rule set of phase 2. Cedar policies remain the plan for several owners sharing an office *(planned)*, see the [governance design](../../docs/design/governance.md).

## Where it sits

- Depends on: `serde`, `serde_json`, `thiserror`, `toml`. No async, no I/O beyond parsing a string.
- Used by: the server, which evaluates every tool call a harness or the native loop wants to make and turns `Ask` into an approval request on the owner's phone. The verdict is recorded as a `PolicyDecision` event.

## Public API

| Item | What it is |
|---|---|
| `Decision` | `Allow`, `Ask`, `Deny` (TOML: `"allow"`, `"ask"`, `"deny"`). `Decision::most_restrictive(a, b)` ranks deny over ask over allow. |
| `PermissionRequest<'a> { tool, input, repo }` | The call to judge: the harness's tool name, its JSON input, and an optional repository label. |
| `Rule { tool, command, path, repo, decision, reason }` | One rule. Every condition that is present must hold. |
| `RuleSet { default, rules }` | The file. `from_toml`, `to_toml`, `defaults()`, `evaluate(&req) -> Verdict`, `summarize(&req) -> String`. |
| `Verdict { decision, rule, reason }` | `rule` is the 0-based index of the deciding rule (`None` for the default); `reason` names it 1-based, as in the file. |
| `RulesError` | `Toml(toml::de::Error)` from `from_toml`. |
| `glob_matches(pattern, text)` | The anchored glob used for every condition: `*` any run, `?` one character, case-sensitive. |

## Using it

```rust
use shiioo_rules::{Decision, PermissionRequest, RuleSet};

let rules = RuleSet::defaults();
let input = serde_json::json!({ "command": "git status && git push origin shiioo/b619ff8d" });
let verdict = rules.evaluate(&PermissionRequest { tool: "Bash", input: &input, repo: None });
assert_eq!(verdict.decision, Decision::Ask);
assert!(verdict.reason.starts_with("segment \"git push origin shiioo/b619ff8d\": rule "));

let summary = RuleSet::summarize(&PermissionRequest { tool: "Bash", input: &input, repo: None });
// "Bash: git status && git push origin shiioo/b619ff8d"
```

`git status*` alone would allow, but the `git push *` segment asks, and ask beats allow. The same shape appears in the unit test `chained_commands_take_the_most_restrictive_segment`.

## Testing

```sh
cargo test -p shiioo-rules
```

Twenty unit tests: four for the glob, sixteen for the rule set (TOML round trip, string-or-list `tool`, the default denies and allows, chained commands, piping into a shell, repo- and path-scoped rules, command rules that need a `command`, verdict reasons, summaries).

## Invariants

- A rule's `tool`, `command`, `path` and `repo` are all globs anchored at both ends. `git push` does not match `git push origin x`; write `git push *`.
- A rule with `command` only matches tools whose input has a string `command`. A rule with `repo` only matches requests that carry a label. A rule with `path` looks at `file_path`, `path` or `notebook_path`.
- `default` must be present in the TOML; `rules` may be omitted.
- Shell splitting at `&&`, `||`, `;` and `|` does not understand quotes. A separator inside a string still splits, which can only make a verdict more restrictive.
- `summarize` is for a phone notification: `Tool: detail`, with the detail cut at 120 characters.

Format and the full default rule set: [`docs/rules-format.md`](docs/rules-format.md).

## Further reading

- [Architecture overview](../../docs/architecture.md), [governance design](../../docs/design/governance.md), [security design](../../docs/design/security.md).
- [ADR-0005 Cedar policies and Biscuit credentials](../../docs/adr/0005-cedar-policies-and-biscuit-credentials.md) for where this is going, [ADR-0008 Personal workforce](../../docs/adr/0008-personal-workforce-not-managed-cloud.md) for why one owner is enough today.
