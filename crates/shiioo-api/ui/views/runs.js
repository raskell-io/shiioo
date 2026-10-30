// Home and the new-task sheet. On a phone, home is the run list (what
// needs you, then everything else); on a desk the sidebar is the list, so
// home is a quiet overview.

import { age, api, app, el, esc, kind, mark, MOD, narrow, needsYou, onLeave, showError, who } from "../core.js";
import { allRuns, getCapacity, upsert, watch } from "../office.js";
import { unread, waitingFor, where } from "./side.js";

export function renderHome() {
  if (narrow()) { renderPhoneList(); return; }
  app.innerHTML = "";
  const home = el(`<div class="home"></div>`);
  app.appendChild(home);
  onLeave(watch(() => {
    const runs = allRuns();
    const attn = runs.filter((r) => needsYou(r.status.state));
    const working = runs.filter((r) => r.status.state === "open");
    if (!runs.length) {
      home.innerHTML = `<div class="empty">
        <div class="art" aria-hidden="true"><span class="brand-mark">s</span></div>
        <h2>Nothing running yet</h2>
        <p>Give the office a task. It picks Claude Code or Codex by how much capacity each has left, works on its own branch, and asks you before anything risky.</p>
        <div class="ways">
          <div><b>Here</b><span>New task <kbd>N</kbd></span></div>
          <div><b>Telegram</b><span><code>/run fix the flaky test in sango</code></span></div>
          <div><b>Terminal</b><span><code>shiioo run "…"</code></span></div>
        </div>
        <button class="btn primary" type="button" data-new>New task</button>
      </div>`;
    } else {
      const line = [attn.length ? `${attn.length} ${attn.length === 1 ? "needs" : "need"} you` : null,
        working.length ? `${working.length} working` : null].filter(Boolean).join(" · ") || "All quiet";
      home.innerHTML = `<div class="empty">
        <div class="art" aria-hidden="true"><span class="brand-mark">s</span></div>
        <h2>${esc(line)}</h2>
        <p>${attn.length ? "Open the first one with <kbd>J</kbd>, or pick a run on the left." : "Pick a run on the left, or give the office something new."}</p>
        <div class="actions">${attn.length ? `<a class="btn primary" href="#run=${esc(attn[0].id)}">Next that needs you <kbd>J</kbd></a>` : ""}
          <button class="btn${attn.length ? "" : " primary"}" type="button" data-new>New task <kbd>N</kbd></button></div>
      </div>`;
    }
    home.querySelector("[data-new]")?.addEventListener("click", () => openNewTask());
  }));
}

function renderPhoneList() {
  app.innerHTML = "";
  const top = el(`<div class="ph-top"><a class="brand" href="#">shiioo</a><button class="btn" type="button">New task</button></div>`);
  top.querySelector("button").addEventListener("click", () => openNewTask());
  const list = el(`<div class="ph-list"></div>`);
  app.append(top, list);
  onLeave(watch(() => {
    const runs = allRuns();
    const attn = runs.filter((r) => needsYou(r.status.state));
    const rest = runs.filter((r) => !needsYou(r.status.state));
    const cap = getCapacity().map((p) => `${who(p.provider)} · ${Object.entries(p.windows || {}).map(([n, w]) =>
      `${{ five_hour: "5h", seven_day: "7d" }[n] || n} ${Math.round((w.utilization || 0) * 100)}%`).join(" · ")}`).join("   ");
    const row = (r, attnRow) => {
      const k = kind(r.status.state);
      return `<a class="ph-row${attnRow ? " attn-row" : ""}${k === "done" || k === "cancelled" ? " done" : ""}${unread(r) ? " unread" : ""}" href="#run=${esc(r.id)}">
        ${mark(r.status.state)}<span><span class="t">${esc(attnRow ? waitingFor(r) : r.task || "(no task)")}</span><span class="r">${esc(where(r))}</span></span>
        <span class="m">${esc(age(r.updated_at))}</span></a>`;
    };
    list.innerHTML = `${cap ? `<div class="ph-cap">${esc(cap)}</div>` : ""}
      ${attn.length ? `<h2 class="attn-h">Needs you · ${attn.length}</h2>${attn.map((r) => row(r, true)).join("")}` : ""}
      ${rest.length ? `<h2>Runs</h2>${rest.map((r) => row(r, false)).join("")}` : ""}
      ${runs.length ? "" : `<div class="empty"><h2>Nothing running yet</h2><p>Tap New task to give the office something to do.</p></div>`}`;
  }));
}

/** The new-task sheet: a task, optionally a repository and who does it. */
export function openNewTask() {
  if (document.querySelector(".scrim")) return;
  const repos = [...new Set(allRuns().map((r) => r.repo).filter(Boolean))];
  const scrim = el(`<div class="scrim" role="dialog" aria-modal="true" aria-labelledby="nt-h">
    <form class="sheet">
      <h2 id="nt-h">New task</h2>
      <p class="sub">The office runs it on its own branch and asks you before anything risky.</p>
      <div class="field"><label for="nt-task">What should be done?</label>
        <textarea id="nt-task" required placeholder="Fix the flaky test in tests/net.rs and open a PR"></textarea></div>
      <div class="field"><label for="nt-repo">Repository</label>
        <input id="nt-repo" list="nt-repos" placeholder="A configured label or an absolute path (optional)" autocapitalize="off" autocorrect="off" spellcheck="false">
        <datalist id="nt-repos">${repos.map((r) => `<option value="${esc(r)}">`).join("")}</datalist></div>
      <div class="field"><label for="nt-who">Who</label>
        <select id="nt-who"><option value="auto">Whoever has the most capacity left</option><option value="claude">Claude Code</option><option value="codex">Codex</option></select></div>
      <div class="row"><span class="note">${MOD}↵ to start · Esc to close</span>
        <button class="btn quiet" type="button" data-close>Cancel</button>
        <button class="btn primary" type="submit">Start</button></div>
    </form></div>`);
  const form = scrim.querySelector("form");
  const close = () => { scrim.remove(); document.removeEventListener("keydown", onKey, true); };
  const onKey = (e) => {
    if (e.key === "Escape") { e.preventDefault(); close(); }
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); form.requestSubmit(); }
  };
  document.addEventListener("keydown", onKey, true);
  scrim.addEventListener("click", (e) => { if (e.target === scrim) close(); });
  scrim.querySelector("[data-close]").addEventListener("click", close);
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    const task = form.querySelector("#nt-task").value.trim();
    const repo = form.querySelector("#nt-repo").value.trim();
    const employee = form.querySelector("#nt-who").value;
    if (!task) return;
    const start = form.querySelector("[type=submit]");
    start.disabled = true;
    try {
      const run = await api("/v1/runs", { method: "POST", body: JSON.stringify({ task, employee, ...(repo ? { repo } : {}) }) });
      upsert(run);
      close();
      location.hash = `run=${run.id}`;
    } catch (err) { showError(err); start.disabled = false; }
  });
  document.body.appendChild(scrim);
  form.querySelector("#nt-task").focus();
}

