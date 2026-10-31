// The sidebar: what needs you, then every run by repository, then the
// providers' capacity. Quiet by design: only "needs you" is coloured, and a
// finished run you have not opened yet carries a small dot.

import { age, el, esc, isOpen, kind, mark, MOD, needsYou, setTheme, side, theme, trunc, who } from "../core.js";
import { allRuns, getCapacity, isLive, watch } from "../office.js";
import { openNewTask } from "./runs.js";

const SEEN_KEY = "shiioo_seen";
const SINCE_KEY = "shiioo_since";
const PER_SPACE = 6;
let current = null;
let expanded = new Set();
let seen = {};
try { seen = JSON.parse(localStorage.getItem(SEEN_KEY) || "{}"); } catch (_) { seen = {}; }
if (!localStorage.getItem(SINCE_KEY)) localStorage.setItem(SINCE_KEY, String(Date.now()));
const since = Number(localStorage.getItem(SINCE_KEY));
let paint = () => {};

/** The run shown in the main pane, highlighted here and marked read. */
export function setCurrent(id) {
  current = id;
  paint();
}

/** Finished since you last looked at it. */
export function unread(r) {
  return r.id !== current && !isOpen(r.status.state) && r.updated_at > (seen[r.id] ?? since);
}

function markSeen(r) {
  if ((seen[r.id] ?? 0) >= r.updated_at) return;
  seen[r.id] = r.updated_at;
  // Only finished runs need remembering; keep the newest few hundred.
  const ids = Object.keys(seen);
  if (ids.length > 400) for (const id of ids.slice(0, ids.length - 400)) delete seen[id];
  localStorage.setItem(SEEN_KEY, JSON.stringify(seen));
}

/** "sango · Claude Code": where and who, for a second line. */
export const where = (r) => [r.repo, who(r.runtime)].filter(Boolean).join(" · ");

/** What a waiting run is waiting for, in a few words. */
export function waitingFor(r) {
  const st = r.status;
  if (st.state === "waiting_approval") return st.summary || "Approval";
  if (st.state === "parked") return r.task;
  if (st.state === "needs_attention") return r.task;
  return r.task;
}

export function renderSide() {
  side.innerHTML = "";
  const top = el(`<div class="side-top"><a class="brand" href="#">shiioo<small>office</small></a></div>`);
  const newBtn = el(`<button class="new-btn" type="button"><span>New task</span><kbd>N</kbd></button>`);
  newBtn.addEventListener("click", () => openNewTask());
  const scroll = el(`<div class="side-scroll"></div>`);
  const foot = el(`<div class="side-foot"></div>`);
  side.append(top, newBtn, scroll, foot);

  scroll.addEventListener("click", (e) => {
    const more = e.target.closest("[data-more]");
    if (more) { expanded.add(more.dataset.more); paint(); }
  });

  paint = () => {
    const runs = allRuns();
    const cur = runs.find((r) => r.id === current);
    if (cur) markSeen(cur);
    const attn = runs.filter((r) => needsYou(r.status.state));
    const rest = runs.filter((r) => !needsYou(r.status.state));

    let html = "";
    if (attn.length) {
      html += `<section class="group attn"><div class="group-h">Needs you<span class="count">${attn.length}</span></div>`;
      for (const r of attn) {
        html += `<a class="item attn-item" href="#run=${esc(r.id)}"${r.id === current ? ' aria-current="page"' : ""}>
          ${mark(r.status.state)}<span><span class="t">${esc(trunc(waitingFor(r), 80))}</span><span class="sub">${esc(where(r))}</span></span>
          <span class="m">${esc(age(r.updated_at))}</span></a>`;
      }
      html += `</section>`;
    }
    const spaces = new Map();
    for (const r of rest) {
      const key = r.repo || "";
      if (!spaces.has(key)) spaces.set(key, []);
      spaces.get(key).push(r);
    }
    if (spaces.size) {
      html += `<section class="group"><div class="group-h">Runs</div>`;
      // Spaces in order of their latest activity; runs without a repo last.
      const order = [...spaces.keys()].sort((a, b) => (a === "") - (b === "") || spaces.get(b)[0].updated_at - spaces.get(a)[0].updated_at);
      for (const key of order) {
        const list = spaces.get(key);
        const working = list.filter((r) => isOpen(r.status.state)).length;
        html += `<div class="space-h">${esc(key || "Office")}<span class="m">${working ? `${working} working` : ""}</span></div>`;
        const shown = expanded.has(key) ? list : list.slice(0, PER_SPACE);
        // Keep the current run visible even when it is beyond the cut.
        if (!shown.some((r) => r.id === current) && list.some((r) => r.id === current)) shown.push(list.find((r) => r.id === current));
        for (const r of shown) {
          const k = kind(r.status.state);
          html += `<a class="item ${k === "done" || k === "cancelled" ? "done" : ""}${unread(r) ? " unread" : ""}" href="#run=${esc(r.id)}"
            ${r.id === current ? 'aria-current="page"' : ""} title="${esc(r.task)}">
            ${mark(r.status.state)}<span class="t">${esc(r.task || "(no task)")}</span><span class="m">${esc(age(r.updated_at))}</span></a>`;
        }
        if (shown.length < list.length) html += `<button class="more" type="button" data-more="${esc(key)}">Show ${list.length - shown.length} more</button>`;
      }
      html += `</section>`;
    }
    if (!runs.length) html += `<p class="item done" style="display:block;margin-top:16px">No runs yet.</p>`;
    scroll.innerHTML = html;

    // Footer: capacity, connection, theme.
    const meters = [];
    for (const p of getCapacity()) {
      for (const [name, w] of Object.entries(p.windows || {})) {
        const pct = Math.round((w.utilization || 0) * 100);
        const label = `${{ "claude-code": "Claude", codex: "Codex" }[p.provider] || p.provider} ${{ five_hour: "5h", seven_day: "7d" }[name] || name}`;
        meters.push(`<div class="meter" title="${esc(label)}: ${pct}% used"><span>${esc(label)}</span>
          <span class="bar"><i class="${pct >= 80 ? "high" : ""}" style="width:${pct}%"></i></span><span>${pct}%</span></div>`);
      }
    }
    const t = theme();
    foot.innerHTML = `${meters.join("")}
      <div class="foot-row"><span class="live ${isLive() ? "" : "off"}"><i></i>${isLive() ? "Live" : "Reconnecting…"}</span><span class="grow"></span>
      <span class="seg" role="group" aria-label="Theme">${["system", "light", "dark"].map((v) =>
        `<button type="button" data-theme="${v}" aria-pressed="${t === v}">${v[0].toUpperCase() + v.slice(1)}</button>`).join("")}</span></div>`;
    for (const b of foot.querySelectorAll("[data-theme]")) b.addEventListener("click", () => { setTheme(b.dataset.theme); paint(); });
    document.title = attn.length ? `(${attn.length}) Shiioo` : "Shiioo";
  };
  watch(paint);
  newBtn.title = `New task (N) · Next that needs you (J) · ${MOD}↵ sends`;
}
