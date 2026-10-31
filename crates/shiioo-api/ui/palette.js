// The command palette (⌘K): one box that finds runs, repositories and
// commands by fuzzy match. Empty, it shows what needs you, what you can do
// to the run in front of you, and recent runs. Enter opens or runs the
// highlighted item; ⇧ Enter opens a run beside the current pane.
// Also the keyboard cheat sheet (?), built from the same command list.

import { available, COMMANDS, GROUPS, runLine, titleOf } from "./commands.js";
import { age, el, esc, isOpen, mark, MOD, narrow, needsYou, stateLabel, trunc } from "./core.js";
import * as desk from "./desk.js";
import { allRuns } from "./office.js";
import { openNewTask } from "./views/runs.js";

const LIMIT = 40;

/**
 * Fuzzy match of `query` in `text`: every query character in order. The
 * score favours word starts, runs of consecutive characters and an early
 * first match; a plain substring beats any scatter. Returns
 * `{ score, at: [indices] }` or null.
 */
export function fuzzy(query, text) {
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  if (!q) return { score: 0, at: [] };
  const sub = t.indexOf(q);
  if (sub >= 0) {
    const start = sub === 0 || /[\s/._\-:·]/.test(t[sub - 1]);
    return { score: 1000 - sub + (start ? 200 : 0) + q.length * 10, at: [...q].map((_, i) => sub + i) };
  }
  const at = [];
  let score = 0;
  let prev = -2;
  let from = 0;
  for (const ch of q) {
    if (ch === " ") continue;
    // Prefer this character at a word start after `from`, else the next one.
    let i = -1;
    for (let k = from; k < t.length; k++) {
      if (t[k] !== ch) continue;
      if (i < 0) i = k;
      if (k === 0 || /[\s/._\-:·]/.test(t[k - 1])) { i = k; break; }
    }
    if (i < 0) return null;
    const wordStart = i === 0 || /[\s/._\-:·]/.test(t[i - 1]);
    score += 10 + (wordStart ? 25 : 0) + (i === prev + 1 ? 15 : 0) - Math.min(i - prev - 1, 10);
    at.push(i);
    prev = i;
    from = i + 1;
  }
  // Letters strewn across a long text are noise, not a match: the hits
  // must sit close together unless each starts a word ("ffr" for
  // "fix flaky retry").
  const starts = at.filter((i) => i === 0 || /[\s/._\-:·]/.test(t[i - 1])).length;
  if (at[at.length - 1] - at[0] + 1 > Math.max(q.length * 3, q.length + 6) && starts < at.length) return null;
  return { score: score - at[0], at };
}

/** `text` with the matched characters marked. */
function marked(text, at) {
  if (!at?.length) return esc(text);
  const set = new Set(at);
  let out = "";
  let open = false;
  for (let i = 0; i < text.length; i++) {
    const hit = set.has(i);
    if (hit && !open) { out += "<mark>"; open = true; }
    if (!hit && open) { out += "</mark>"; open = false; }
    out += esc(text[i]);
  }
  return out + (open ? "</mark>" : "");
}

/** Everything the palette can offer right now. */
function items() {
  const out = [];
  const runs = allRuns();
  const onDesk = !narrow();
  for (const c of COMMANDS) {
    if (c.hidden || !c.run || (c.desk && !onDesk) || !available(c)) continue;
    out.push({ kind: "cmd", cmd: c, text: titleOf(c), group: c.group, key: c.label, attention: c.attention, confirm: c.confirm });
  }
  for (const r of runs) {
    out.push({ kind: "run", run: r, text: r.task || "(no task)", sub: `${runLine(r)}${r.branch ? ` · ${r.branch}` : ""} · ${r.id.slice(0, 8)}`, state: r.status.state });
  }
  for (const repo of [...new Set(runs.map((r) => r.repo).filter(Boolean))]) {
    out.push({ kind: "repo", repo, text: `New task in ${repo}`, group: "Office" });
  }
  return out;
}

function rank(query) {
  const all = items();
  if (!query.trim()) {
    // A starting point: what needs you, this run, recent runs, the rest.
    const attn = all.filter((i) => i.kind === "run" && needsYou(i.state)).map((i) => ({ ...i, section: "Needs you" }));
    const here = all.filter((i) => i.kind === "cmd" && i.group === "This run").map((i) => ({ ...i, section: "This run" }));
    const open = new Set(desk.tabs());
    const recent = all.filter((i) => i.kind === "run" && !needsYou(i.state))
      .sort((a, b) => (open.has(b.run.id) - open.has(a.run.id)) || b.run.updated_at - a.run.updated_at)
      .slice(0, 6).map((i) => ({ ...i, section: open.has(i.run.id) ? "Open tabs" : "Recent" }));
    const rest = all.filter((i) => i.kind === "cmd" && i.group !== "This run").map((i) => ({ ...i, section: i.group }));
    return [...attn, ...here, ...recent, ...rest];
  }
  const scored = [];
  for (const i of all) {
    // A run also matches by repository, branch and id.
    const m = fuzzy(query, i.text);
    const alt = i.kind === "run" ? fuzzy(query, i.sub) : null;
    const best = m && (!alt || m.score >= alt.score - 50) ? { ...m, onSub: false } : alt ? { ...alt, onSub: true, score: alt.score - 50 } : null;
    if (!best) continue;
    // Small nudges: what needs you and open work first, finished last.
    let bonus = 0;
    if (i.kind === "run") bonus = needsYou(i.state) ? 60 : isOpen(i.state) ? 25 : 0;
    if (i.kind === "cmd" && i.attention) bonus = 40;
    scored.push({ ...i, score: best.score + bonus, at: best.onSub ? null : best.at, subAt: best.onSub ? best.at : null });
  }
  return scored.sort((a, b) => b.score - a.score).slice(0, LIMIT);
}

export function openPalette() {
  if (document.querySelector(".scrim")) return;
  const scrim = el(`<div class="scrim palette-scrim" role="dialog" aria-modal="true" aria-label="Command palette">
    <div class="palette">
      <div class="pal-in"><svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true"><circle cx="7" cy="7" r="4.6" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M10.4 10.4 14 14" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/></svg>
        <input type="text" placeholder="Search runs, repositories and commands" aria-label="Search" autocomplete="off" autocapitalize="off" spellcheck="false"
          role="combobox" aria-expanded="true" aria-controls="pal-list" aria-autocomplete="list"><kbd>Esc</kbd></div>
      <div class="pal-list" id="pal-list" role="listbox"></div>
      <div class="pal-foot"><span><kbd>↑</kbd><kbd>↓</kbd> move</span><span><kbd>↵</kbd> open</span>${narrow() ? "" : `<span><kbd>⇧ ↵</kbd> open beside</span>`}<span class="grow"></span><span><kbd>?</kbd> all shortcuts</span></div>
    </div></div>`);
  const input = scrim.querySelector("input");
  const list = scrim.querySelector(".pal-list");
  const previous = document.activeElement;
  let shown = [];
  let sel = 0;
  let armed = null; // a command waiting for its second Enter

  const close = (restore = true) => {
    scrim.remove();
    document.removeEventListener("keydown", onKey, true);
    if (restore && previous instanceof HTMLElement && previous.isConnected) previous.focus({ preventScroll: true });
  };
  const paint = () => {
    shown = rank(input.value);
    sel = Math.min(sel, Math.max(0, shown.length - 1));
    let html = "";
    let section = null;
    shown.forEach((i, n) => {
      const head = i.section || (input.value.trim() ? null : i.group);
      if (head && head !== section) { html += `<div class="pal-sec">${esc(head)}</div>`; section = head; }
      const id = `pal-${n}`;
      const selected = n === sel ? ' aria-selected="true"' : "";
      if (i.kind === "run") {
        const r = i.run;
        html += `<div class="pal-item" role="option" id="${id}" data-n="${n}"${selected}>${mark(r.status.state)}
          <span class="pal-t"><span class="t">${i.at ? marked(i.text, i.at) : esc(trunc(i.text, 120))}</span>
          <span class="s">${i.subAt ? marked(i.sub, i.subAt) : esc(i.sub)}</span></span>
          <span class="pal-k">${needsYou(r.status.state) ? `<span class="pill waiting">${esc(stateLabel(r.status.state))}</span>` : esc(age(r.updated_at))}</span></div>`;
      } else {
        const confirm = armed === i.cmd?.id;
        html += `<div class="pal-item cmd${i.attention ? " attn" : ""}${confirm ? " armed" : ""}" role="option" id="${id}" data-n="${n}"${selected}>
          <span class="pal-ic" aria-hidden="true">${i.kind === "repo" ? "+" : i.attention ? "!" : "›"}</span>
          <span class="pal-t"><span class="t">${confirm ? esc(i.confirm) : i.at ? marked(i.text, i.at) : esc(trunc(i.text, 120))}</span></span>
          <span class="pal-k">${i.key ? `<kbd>${esc(i.key)}</kbd>` : ""}</span></div>`;
      }
    });
    if (!shown.length) html = `<div class="pal-empty">Nothing matches “${esc(input.value)}”. <button type="button" class="link" data-as-task>Start it as a new task</button></div>`;
    list.innerHTML = html;
    list.querySelector("[data-as-task]")?.addEventListener("click", () => { const t = input.value; close(false); openNewTask(); setTimeout(() => { const f = document.querySelector("#nt-task"); if (f) f.value = t; }, 0); });
    input.setAttribute("aria-activedescendant", shown.length ? `pal-${sel}` : "");
    list.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
  };
  const choose = (n, beside) => {
    const i = shown[n];
    if (!i) return;
    if (i.kind === "run") { close(false); if (beside) desk.showBeside(i.run.id); else desk.show(i.run.id); desk.focusedView()?.focus(); return; }
    if (i.kind === "repo") { close(false); openNewTask({ repo: i.repo }); return; }
    if (i.confirm && armed !== i.cmd.id) { armed = i.cmd.id; paint(); return; }
    close();
    i.cmd.run();
  };
  const onKey = (e) => {
    if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); if (armed) { armed = null; paint(); } else close(); return; }
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") { e.preventDefault(); close(); return; }
    if (e.key === "ArrowDown" || (e.ctrlKey && e.key === "n")) { e.preventDefault(); sel = (sel + 1) % Math.max(shown.length, 1); armed = null; paint(); }
    else if (e.key === "ArrowUp" || (e.ctrlKey && e.key === "p")) { e.preventDefault(); sel = (sel - 1 + shown.length) % Math.max(shown.length, 1); armed = null; paint(); }
    else if (e.key === "Enter") { e.preventDefault(); choose(sel, e.shiftKey); }
    else if (e.key === "?" && !input.value) { e.preventDefault(); close(false); openHelp(); }
  };
  input.addEventListener("input", () => { sel = 0; armed = null; paint(); });
  list.addEventListener("mousemove", (e) => { const it = e.target.closest(".pal-item"); if (it && Number(it.dataset.n) !== sel) { sel = Number(it.dataset.n); armed = null; for (const x of list.querySelectorAll(".pal-item")) x.toggleAttribute("aria-selected", x === it); } });
  list.addEventListener("click", (e) => { const it = e.target.closest(".pal-item"); if (it) choose(Number(it.dataset.n), e.shiftKey); });
  scrim.addEventListener("mousedown", (e) => { if (e.target === scrim) close(); });
  document.addEventListener("keydown", onKey, true);
  document.body.appendChild(scrim);
  paint();
  input.focus();
}

/** Every key, grouped, from the command list. */
export function openHelp() {
  if (document.querySelector(".scrim")) return;
  const onDesk = !narrow();
  const groups = GROUPS.map((g) => {
    const rows = COMMANDS.filter((c) => c.group === g && c.label && (!c.desk || onDesk))
      .map((c) => `<div class="key-row"><span>${esc(c.help || titleOf(c))}</span><kbd>${esc(c.label)}</kbd></div>`);
    if (g === "This run") rows.push(`<div class="key-row"><span>Send the message</span><kbd>${esc(MOD)} ↵</kbd></div>`,
      `<div class="key-row"><span>Send it now (stop the current step)</span><kbd>${esc(MOD)} ⇧ ↵</kbd></div>`,
      `<div class="key-row"><span>Close the panel, leave the text box</span><kbd>Esc</kbd></div>`);
    if (g === "Tabs and panes" && onDesk) rows.push(`<div class="key-row"><span>Open a run beside this pane</span><kbd>⇧ click</kbd></div>`,
      `<div class="key-row"><span>Close a tab</span><kbd>middle click</kbd></div>`);
    return rows.length ? `<section><h3>${esc(g)}</h3>${rows.join("")}</section>` : "";
  }).join("");
  const scrim = el(`<div class="scrim" role="dialog" aria-modal="true" aria-labelledby="help-h">
    <div class="sheet help"><div class="help-h"><h2 id="help-h">Keyboard</h2><button class="icon-btn" type="button" data-close aria-label="Close">✕</button></div>
      <p class="sub">Single keys work when no text box has the cursor.</p>
      <div class="keys">${groups}</div></div></div>`);
  const close = () => { scrim.remove(); document.removeEventListener("keydown", onKey, true); };
  const onKey = (e) => { if (e.key === "Escape" || e.key === "?") { e.preventDefault(); e.stopPropagation(); close(); } };
  scrim.addEventListener("mousedown", (e) => { if (e.target === scrim) close(); });
  scrim.querySelector("[data-close]").addEventListener("click", close);
  document.addEventListener("keydown", onKey, true);
  document.body.appendChild(scrim);
  scrim.querySelector("[data-close]").focus();
}
