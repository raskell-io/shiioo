// The component kit (#kit): every piece of the desk in both themes side by
// side, drawn by the same code the desk uses (the log is the real log fed
// with sample events, the diff the real diff), and a live contrast check
// of the text colours against the surfaces they sit on.

import { el, esc, mark, MOD, stateLabel } from "../core.js";
import { renderDiff } from "./diff.js";
import { createLog } from "./log.js";

/** Text colour on surface, as the CSS uses them; AA needs 4.5:1. */
export const PAIRS = [
  ["ink", "bg"], ["ink", "panel"], ["ink", "sunk"],
  ["muted", "bg"], ["muted", "panel"], ["muted", "sunk"],
  ["faint", "bg"], ["faint", "panel"], ["faint", "sunk"],
  ["accent", "bg"], ["accent", "panel"], ["accent", "accent-soft"], ["accent-ink", "accent"],
  ["attn-ink", "attn-soft"], ["attn-ink", "sunk"], ["attn-ink", "panel"],
  ["ok", "panel"], ["bad", "panel"], ["bad", "bad-soft"], ["panel", "ink"],
  ["ink", "frame"], ["muted", "frame"], ["ink", "side"], ["muted", "side"], ["faint", "side"],
  ["ink", "raise"], ["muted", "raise"], ["faint", "raise"], ["attn-ink", "raise"],
];
const TOKENS = ["frame", "bg", "panel", "side", "raise", "sunk", "ink", "muted", "faint", "line", "accent", "accent-soft", "attn", "attn-soft", "attn-ink", "ok", "bad", "bad-soft", "add", "del"];
const STATES = ["open", "waiting_approval", "parked", "needs_attention", "completed", "failed", "cancelled"];

const DIFF = `diff --git a/src/retry.rs b/src/retry.rs
--- a/src/retry.rs
+++ b/src/retry.rs
@@ -12,7 +12,9 @@ pub fn fetch(url: &str) -> Result<Body> {
     let client = Client::new();
-    for _ in 0..3 {
-        if let Ok(b) = client.get(url) { return Ok(b); }
+    backoff::retry(Policy::default(), || {
+        client.get(url).map_err(Transient::from)
+    })
-    }
-    Err(Error::GaveUp)
 }
diff --git a/tests/net.rs b/tests/net.rs
--- a/tests/net.rs
+++ b/tests/net.rs
@@ -40,3 +40,4 @@ fn flaky_fetch_recovers() {
     let server = Mock::failing_twice();
     assert!(fetch(&server.url()).is_ok());
+    assert_eq!(server.hits(), 3);
 }
`;

function events() {
  const t0 = Date.now() - 9 * 60000;
  let seq = 0;
  const ev = (payload, min) => ({ seq: seq++, recorded_at: t0 + min * 60000, payload });
  return [
    ev({ type: "run_created", spec: { task: "Fix the flaky test in tests/net.rs", runtime: { kind: "harness", harness: "claude-code" } } }, 0),
    ev({ type: "sandbox_started", level: "worktree", detail: {} }, 0),
    ev({ type: "harness_session_started", harness: "claude-code", session_id: "8241059a", model: "claude-sonnet-5-5" }, 0),
    ev({ type: "model_response_recorded", response: { content: [{ type: "text", text: "The test retries **three times** without waiting. I'll use the existing `backoff` helper instead of a hand-rolled loop." }] } }, 1),
    ev({ type: "tool_intent", call_id: "c1", tool: "Bash", input: { command: "cargo test -p net flaky_fetch" } }, 1),
    ev({ type: "tool_result", call_id: "c1", is_error: true, output: "running 1 test\ntest flaky_fetch_recovers ... FAILED\n\nfailures:\n    thread panicked at tests/net.rs:42\n\ntest result: FAILED. 0 passed; 1 failed" }, 2),
    ev({ type: "tool_intent", call_id: "c2", tool: "Edit", input: { file_path: "src/retry.rs" } }, 3),
    ev({ type: "tool_result", call_id: "c2", is_error: false, output: "ok" }, 3),
    ev({ type: "user_message", author: "owner", content: "Use the existing backoff helper, don't add a dependency." }, 4),
    ev({ type: "interrupt_requested", by: "owner" }, 4),
    ev({ type: "tool_intent", call_id: "c3", tool: "Bash", input: { command: "git push origin shiioo/5568cd6c" } }, 6),
    ev({ type: "policy_decision", tool: "Bash", decision: "ask", reason: "git push asks the owner" }, 6),
    ev({ type: "approval_requested", approval_id: "kit-a1", tool: "Bash", summary: "Bash: git push origin shiioo/5568cd6c", input: {} }, 6),
  ];
}

function column(themeName, title) {
  const col = el(`<section class="kit-col t-${themeName}" aria-label="${esc(title)}"><h2>${esc(title)}</h2></section>`);
  const sec = (name, body) => { const s = el(`<div class="kit-sec"><h3>${esc(name)}</h3></div>`); s.appendChild(typeof body === "string" ? el(body) : body); col.appendChild(s); return s; };

  sec("Colour", `<div class="swatches">${TOKENS.map((t) => `<div class="swatch"><i style="background:var(--${t})"></i><span>--${t}</span></div>`).join("")}</div>`);
  const table = sec("Contrast (AA needs 4.5:1)", `<div class="kit-box"><table class="contrast"><thead><tr><th>Text</th><th>On</th><th>Sample</th><th>Ratio</th><th></th></tr></thead><tbody></tbody></table></div>`);

  sec("Type", `<div class="kit-box type-scale">
    ${[["2xl", "Nothing running yet"], ["xl", "Fix the flaky test in tests/net.rs"], ["lg", "Approval and diff"], ["md", "The office runs it on its own branch."], ["sm", "sango · Claude Code · 4 min"], ["xs", "NEEDS YOU"]]
      .map(([k, t]) => `<div><code>--fs-${k}</code><span style="font-size:var(--fs-${k})">${t}</span></div>`).join("")}
    <div><code>brand</code><span style="font:700 22px/1 var(--font-brand)">shiioo</span></div>
    <div><code>mono</code><span class="mono">cargo test -p net</span></div></div>`);

  sec("Buttons", `<div class="kit-box"><div class="kit-row">
    <button class="btn primary" type="button">Send at next turn <kbd>${MOD}↵</kbd></button>
    <button class="btn" type="button">Now <kbd>${MOD}⇧↵</kbd></button>
    <button class="btn quiet" type="button">Raw</button>
    <button class="btn allow" type="button">Allow</button>
    <button class="btn armed" type="button">Really cancel?</button>
    <button class="btn" type="button" aria-pressed="true">Approval &amp; diff <kbd>D</kbd></button>
    <button class="btn" type="button" disabled>Disabled</button>
    <button class="icon-btn" type="button" aria-label="Close">✕</button>
    <span class="seg" role="group"><button type="button" aria-pressed="true">System</button><button type="button" aria-pressed="false">Light</button><button type="button" aria-pressed="false">Dark</button></span>
    <button class="palette-btn" type="button"><span>Search</span><kbd>${MOD}K</kbd></button></div></div>`);

  sec("States", `<div class="kit-box"><div class="kit-row">${STATES.map((s) => `<span class="pill ${["open"].includes(s) ? "working" : ["waiting_approval", "parked", "needs_attention"].includes(s) ? "waiting" : s === "completed" ? "done" : s}">${mark(s)}${esc(stateLabel(s))}</span>`).join("")}</div>
    <div class="kit-row" style="margin-top:12px">${STATES.map((s) => `<span title="${esc(s)}" style="display:inline-grid;place-items:center;width:24px;height:24px">${mark(s)}</span>`).join("")}</div></div>`);

  sec("Tabs", `<div class="kit-tabbar"><div class="tabbar"><div class="tabs">
    <div class="tab focused">${mark("open")}<span class="t">Fix the flaky test</span><button class="x" type="button">×</button></div>
    <div class="tab shown">${mark("completed")}<span class="t">Bump dependencies</span><button class="x" type="button">×</button></div>
    <div class="tab attn">${mark("waiting_approval")}<span class="t">git push origin…</span><button class="x" type="button">×</button></div>
    <div class="tab unread">${mark("completed")}<span class="t">Docs pass</span><button class="x" type="button">×</button></div></div>
    <div class="bar-tools"><button class="palette-btn" type="button"><span>Search</span><kbd>${MOD}K</kbd></button></div></div></div>`);

  sec("Sidebar", `<div class="kit-box sunk" style="padding:8px"><section class="group attn"><div class="group-h">Needs you<span class="count">1</span></div>
    <a class="item attn-item" href="#kit">${mark("waiting_approval")}<span><span class="t">Bash: git push origin shiioo/5568cd6c</span><span class="sub">sango · Claude Code</span></span><span class="m">2m</span></a></section>
    <section class="group"><div class="space-h">sango<span class="m">1 working</span></div>
    <a class="item" href="#kit" aria-current="page">${mark("open")}<span class="t">Fix the flaky test</span><span class="m">now</span></a>
    <a class="item done unread" href="#kit">${mark("completed")}<span class="t">Docs pass</span><span class="m">3h</span></a>
    <a class="item done" href="#kit">${mark("failed")}<span class="t">Bump dependencies</span><span class="m">1d</span></a></section>
    <div class="side-foot" style="border:0;padding:12px 8px 4px"><div class="meter"><span>Claude 5h</span><span class="bar"><i style="width:42%"></i></span><span>42%</span></div>
    <div class="meter"><span>Codex 7d</span><span class="bar"><i class="high" style="width:86%"></i></span><span>86%</span></div>
    <div class="foot-row"><span class="live"><i></i>Live</span><span class="live off"><i></i>Reconnecting…</span></div></div></div>`);

  const logBox = el(`<div class="kit-box" style="padding:20px 20px 4px"></div>`);
  sec("Log", logBox);
  const log = createLog(logBox, { onDecide: () => {} });
  for (const e of events()) log.add(e);

  sec("Approval", `<div class="card ask"><div class="eyebrow">Needs you</div><h3>Claude Code wants to go ahead</h3>
    <div class="cmd">Bash: git push origin shiioo/5568cd6c</div>
    <div class="row"><button class="btn" type="button">Deny</button><button class="btn allow" type="button">Allow</button></div>
    <div class="foot">Also on your phone. The first answer wins.</div></div>`);
  const diff = el(`<div class="diff-slot"></div>`);
  sec("Diff", diff);
  renderDiff(diff, DIFF);

  sec("Message box", `<div class="kit-box" style="padding:0;overflow:hidden;background:var(--panel)"><form class="composer" onsubmit="return false">
    <label class="hint">Tell Claude Code something. It reads it when it finishes its current step, or at once with Now.</label>
    <div class="box"><textarea rows="1" placeholder="e.g. use the existing helper, don't add a dependency"></textarea>
    <div class="row"><span class="note">Goes into the run's log. The rules still apply.</span>
      <button class="btn" type="button">Now <kbd>${MOD}⇧↵</kbd></button><button class="btn primary" type="button">Send at next turn <kbd>${MOD}↵</kbd></button></div></div></form></div>`);

  sec("Palette", `<div class="palette" style="animation:none;max-height:none"><div class="pal-in"><input type="text" value="flak" aria-label="Sample search"><kbd>Esc</kbd></div>
    <div class="pal-list"><div class="pal-sec">Needs you</div>
    <div class="pal-item" aria-selected="true">${mark("waiting_approval")}<span class="pal-t"><span class="t">Fix the <mark>flak</mark>y test</span><span class="s">sango · Claude Code · 5568cd6c</span></span><span class="pal-k"><span class="pill waiting">Needs your approval</span></span></div>
    <div class="pal-item cmd attn"><span class="pal-ic">!</span><span class="pal-t"><span class="t">Allow: git push origin shiioo/5568cd6c</span></span><span class="pal-k"></span></div>
    <div class="pal-item cmd"><span class="pal-ic">›</span><span class="pal-t"><span class="t">Split: another pane to the right</span></span><span class="pal-k"><kbd>\\</kbd></span></div>
    <div class="pal-item cmd armed"><span class="pal-ic">›</span><span class="pal-t"><span class="t">Enter again to cancel the run</span></span><span class="pal-k"></span></div></div>
    <div class="pal-foot"><span><kbd>↑</kbd><kbd>↓</kbd> move</span><span><kbd>↵</kbd> open</span><span><kbd>⇧ ↵</kbd> open beside</span></div></div>`);

  sec("Fields", `<div class="kit-box"><div class="field"><label>What should be done?</label><textarea placeholder="Fix the flaky test in tests/net.rs and open a PR"></textarea></div>
    <div class="field"><label>Repository</label><input placeholder="A configured label or an absolute path (optional)"></div></div>`);

  sec("Feedback", `<div class="kit-box"><div class="kit-row"><div class="toast" style="animation:none">Sent</div><div class="toast bad" style="animation:none">The run has already finished</div>
    <button class="jump show" type="button" style="position:static;transform:none">Jump to latest ↓</button></div>
    <div style="display:grid;gap:8px;margin-top:16px"><div class="sk" style="width:60%"></div><div class="sk" style="width:85%"></div><div class="sk" style="width:40%"></div></div></div>`);

  sec("Empty", `<div class="kit-box"><div class="empty" style="margin:0;padding:8px"><div class="art"><span class="brand-mark">s</span></div><h2>Nothing running yet</h2>
    <p>Give the office a task. It works on its own branch and asks you before anything risky.</p><button class="btn primary" type="button">New task <kbd>N</kbd></button></div></div>`);

  return { col, table: table.querySelector("tbody") };
}

/** Relative luminance of a CSS colour the browser has resolved. */
function luminance(css) {
  const probe = document.createElement("canvas").getContext("2d");
  probe.fillStyle = css;
  const hex = probe.fillStyle; // "#rrggbb" for opaque colours
  const c = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((x) => (x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4));
  return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
}
export const ratio = (a, b) => { const [x, y] = [luminance(a), luminance(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };

function fillContrast(col, tbody) {
  const cs = getComputedStyle(col);
  const v = (t) => cs.getPropertyValue(`--${t}`).trim();
  tbody.innerHTML = PAIRS.map(([fg, bg]) => {
    const r = ratio(v(fg), v(bg));
    return `<tr><td><code>--${fg}</code></td><td><code>--${bg}</code></td>
      <td><span class="sample" style="color:var(--${fg});background:var(--${bg})">Aa 12:04</span></td>
      <td>${r.toFixed(2)}</td><td class="${r >= 4.5 ? "pass" : "fail"}">${r >= 4.5 ? "AA" : "fails"}</td></tr>`;
  }).join("");
}

export function createKit() {
  const root = el(`<div class="kit"><header class="kit-head"><h1>Component kit</h1>
    <span class="note" style="color:var(--muted);font-size:var(--fs-sm)">Every component, both themes. The checks below run in your browser.</span>
    <a class="btn" href="#">Back to the desk</a></header><div class="kit-cols"></div></div>`);
  const cols = root.querySelector(".kit-cols");
  const light = column("light", "Light");
  const dark = column("dark", "Dark");
  cols.append(light.col, dark.col);
  requestAnimationFrame(() => { fillContrast(light.col, light.table); fillContrast(dark.col, dark.table); });
  return { root, destroy() { root.remove(); } };
}
