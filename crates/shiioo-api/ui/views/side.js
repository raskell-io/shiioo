// The sidebar: what needs you, then every run by repository, then the
// providers' capacity. Quiet by design: only "needs you" is coloured, and a
// finished run you have not opened yet carries a small dot.

import { age, el, esc, isOpen, kind, mark, MOD, needsYou, rail, setTheme, side, theme, trunc, who } from "../core.js";
import { allRuns, getCapacity, isLive, watch } from "../office.js";
import * as desk from "../desk.js";
import { openNewTask } from "./runs.js";
import { nextNeedsYou } from "../commands.js";

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
  const newBtn = el(`<button class="new-btn" type="button"><svg width="18" height="18" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15.5 10.5V15a1.5 1.5 0 0 1-1.5 1.5H5A1.5 1.5 0 0 1 3.5 15V6A1.5 1.5 0 0 1 5 4.5h4.5"/><path d="m13.2 3.8 3 3L10 13l-3.4.4.4-3.4z"/></svg><span>New task</span><kbd>N</kbd></button>`);
  newBtn.addEventListener("click", () => openNewTask());
  const scroll = el(`<div class="side-scroll"></div>`);
  const foot = el(`<div class="side-foot"></div>`);
  side.append(top, newBtn, scroll, foot);

  scroll.addEventListener("click", (e) => {
    const more = e.target.closest("[data-more]");
    if (more) { expanded.add(more.dataset.more); paint(); return; }
    // Shift-click opens the run beside the focused pane.
    const item = e.target.closest("a.item");
    if (item && e.shiftKey && !e.metaKey && !e.ctrlKey) {
      e.preventDefault();
      desk.showBeside(new URLSearchParams(item.getAttribute("href").slice(1)).get("run"));
    }
  });
  document.addEventListener("shiioo:theme", () => paint());

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
    for (const b of foot.querySelectorAll("[data-theme]")) b.addEventListener("click", () => setTheme(b.dataset.theme));
    document.title = attn.length ? `(${attn.length}) Shiioo` : "Shiioo";
    favicon(attn.length > 0);
  };
  watch(paint);
  newBtn.title = `New task (N) · Search (${MOD}K) · All keys (?)`;
}

/** The tab's icon: the brand mark, with an amber dot while something waits. */
let dotted = null;
function favicon(dot) {
  if (dot === dotted) return;
  dotted = dot;
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" rx="8" fill="#2c746b"/>
    <text x="${dot ? 13 : 16}" y="23" font-family="system-ui,sans-serif" font-size="21" font-weight="700" text-anchor="middle" fill="#fff">s</text>
    ${dot ? `<circle cx="25" cy="7" r="6.5" fill="#e3a64c" stroke="#fff" stroke-width="2"/>` : ""}</svg>`;
  document.getElementById("favicon")?.setAttribute("href", `data:image/svg+xml,${encodeURIComponent(svg)}`);
}

// ---- the rail --------------------------------------------------------------
// The narrow column on the window's frame: the brand, and the few things
// you do from anywhere. Every button has its key in the tooltip.

const ICON = {
  home: '<path d="M3.5 9.5 10 4l6.5 5.5V16a1 1 0 0 1-1 1h-3.5v-4.5h-4V17H4.5a1 1 0 0 1-1-1z"/>',
  new: '<path d="M10 4.5v11M4.5 10h11"/>',
  search: '<circle cx="9" cy="9" r="5"/><path d="m12.8 12.8 3.7 3.7"/>',
  attn: '<path d="M5.5 13.5V9a4.5 4.5 0 0 1 9 0v4.5l1.2 1.5H4.3z"/><path d="M8.3 16.8a1.9 1.9 0 0 0 3.4 0"/>',
  keys: '<rect x="2.5" y="5.5" width="15" height="9.5" rx="2"/><path d="M5.5 8.5h.01M8.5 8.5h.01M11.5 8.5h.01M14.5 8.5h.01M6.5 11.8h7"/>',
  kit: '<rect x="3.5" y="3.5" width="5.5" height="5.5" rx="1.4"/><rect x="11" y="3.5" width="5.5" height="5.5" rx="1.4"/><rect x="3.5" y="11" width="5.5" height="5.5" rx="1.4"/><rect x="11" y="11" width="5.5" height="5.5" rx="1.4"/>',
};
const icon = (name) => `<svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICON[name]}</svg>`;

export function renderRail() {
  rail.innerHTML = `
    <a class="rail-brand" href="#" title="Home (H)" aria-label="Home"><span>s</span></a>
    <button class="rail-btn" type="button" data-act="home" title="Home (H)" aria-label="Home">${icon("home")}</button>
    <button class="rail-btn" type="button" data-act="new" title="New task (N)" aria-label="New task">${icon("new")}</button>
    <button class="rail-btn" type="button" data-act="search" title="Search runs and commands (${MOD}K)" aria-label="Search">${icon("search")}</button>
    <button class="rail-btn attn-btn" type="button" data-act="next" title="Next run that needs you (J)" aria-label="Next run that needs you">${icon("attn")}<i class="rail-dot" hidden></i></button>
    <span class="grow"></span>
    <button class="rail-btn" type="button" data-act="keys" title="Keyboard shortcuts (?)" aria-label="Keyboard shortcuts">${icon("keys")}</button>
    <a class="rail-btn" href="#kit" title="Component kit" aria-label="Component kit">${icon("kit")}</a>`;
  rail.addEventListener("click", (e) => {
    const b = e.target.closest("[data-act]");
    if (!b) return;
    const act = b.dataset.act;
    if (act === "home") desk.show(null);
    if (act === "new") openNewTask();
    if (act === "search") document.dispatchEvent(new CustomEvent("shiioo:palette"));
    if (act === "next") nextNeedsYou();
    if (act === "keys") document.dispatchEvent(new CustomEvent("shiioo:help"));
  });
  watch(() => {
    const n = allRuns().filter((r) => needsYou(r.status.state)).length;
    const dot = rail.querySelector(".rail-dot");
    dot.hidden = n === 0;
    dot.textContent = n > 9 ? "9+" : String(n);
    rail.querySelector(".attn-btn").classList.toggle("on", n > 0);
  });
}
