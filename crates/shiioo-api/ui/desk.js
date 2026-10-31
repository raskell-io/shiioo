// The desk's workspace: open runs as tabs, shown in up to three panes side
// by side. Each pane shows one tab (or home); a tab is in at most one pane.
// Run views stay mounted while their tab is open (the newest few), so
// switching is instant; only the views in a pane stream. The layout is
// kept in localStorage, so a reload comes back to the same desk.
//
// On a phone there are no tabs and one pane: the focused one.

import { app, el, esc, mark, MOD, narrow, needsYou, trunc } from "./core.js";
import { getRun, isLoaded, watch } from "./office.js";
import { createHome } from "./views/runs.js";
import { createRunView } from "./views/run.js";
import { setCurrent, unread } from "./views/side.js";

const KEY = "shiioo_desk";
const MAX_PANES = 3;
const MIN_PANE = 360;  // px; a split that would go below this is refused
const MOUNTED = 8;     // run views kept alive beyond the ones on screen

/** { tabs: [id], panes: [{ run: id | null }], focus: i, sizes: [fraction] } */
let state = load();
const views = new Map(); // id -> run view, oldest use first
const closed = [];       // ids of closed tabs, newest last
let page = null;         // a full-width page over the panes (the kit)
let root = null;
let bar = null;
let area = null;
const slots = [];        // per pane: { el, run, content }

function load() {
  try {
    const s = JSON.parse(localStorage.getItem(KEY) || "null");
    if (s && Array.isArray(s.tabs) && Array.isArray(s.panes) && s.panes.length) {
      const panes = s.panes.slice(0, MAX_PANES).map((p) => ({ run: s.tabs.includes(p.run) ? p.run : null }));
      return { tabs: s.tabs, panes, focus: Math.min(s.focus || 0, panes.length - 1), sizes: fit(s.sizes, panes.length) };
    }
  } catch (_) { /* start fresh */ }
  return { tabs: [], panes: [{ run: null }], focus: 0, sizes: [1] };
}
const save = () => localStorage.setItem(KEY, JSON.stringify(state));
function fit(sizes, n) {
  if (!Array.isArray(sizes) || sizes.length !== n || sizes.some((x) => !(x > 0))) return Array(n).fill(1 / n);
  const sum = sizes.reduce((a, b) => a + b, 0);
  return sizes.map((x) => x / sum);
}

const paneOf = (id) => state.panes.findIndex((p) => p.run === id);
export const focusedRun = () => state.panes[state.focus]?.run ?? null;
export const panes = () => state.panes.length;
export const tabs = () => [...state.tabs];
/** The run view in the focused pane, for keys and the palette. */
export const focusedView = () => views.get(focusedRun()) || null;

// ---- the view cache -------------------------------------------------------

function viewFor(id) {
  let v = views.get(id);
  if (v) { views.delete(id); views.set(id, v); return v; }
  v = createRunView(id);
  views.set(id, v);
  // Forget the oldest views that no pane shows.
  const shown = new Set(state.panes.map((p) => p.run));
  for (const [vid, old] of views) {
    if (views.size <= MOUNTED + shown.size) break;
    if (!shown.has(vid) && vid !== id) { old.destroy(); views.delete(vid); }
  }
  return v;
}

function dropView(id) {
  const v = views.get(id);
  if (v) { v.destroy(); views.delete(id); }
}

// ---- actions --------------------------------------------------------------

/** Shows `id` in pane `at` (default: the focused one), opening its tab. */
export function show(id, at = state.focus) {
  if (id === null) { state.panes[at].run = null; state.focus = at; commit(); return; }
  if (!state.tabs.includes(id)) {
    const after = state.tabs.indexOf(state.panes[at].run);
    state.tabs.splice(after < 0 ? state.tabs.length : after + 1, 0, id);
  }
  const elsewhere = paneOf(id);
  if (elsewhere >= 0 && elsewhere !== at) {
    // Already on screen: a plain open goes there; an explicit pane swaps.
    if (at === state.focus) { state.focus = elsewhere; commit(); return; }
    state.panes[elsewhere].run = state.panes[at].run;
  }
  state.panes[at].run = id;
  state.focus = at;
  commit();
}

/** Opens `id` next to the focused pane, splitting if there is room. */
export function showBeside(id) {
  if (narrow()) { show(id); return; }
  const other = paneOf(id);
  if (other >= 0) { focusPane(other); return; }
  if (state.panes.length === 1 && !split(false)) { show(id); return; }
  const at = state.panes.length === 1 ? 1 : (state.focus + 1) % state.panes.length;
  show(id, at);
}

/** Closes a tab (default: the focused pane's). Its pane shows a neighbour. */
export function closeTab(id = focusedRun()) {
  if (!id) { if (state.panes.length > 1) closePane(); return; }
  const i = state.tabs.indexOf(id);
  if (i < 0) return;
  state.tabs.splice(i, 1);
  closed.push(id);
  if (closed.length > 20) closed.shift();
  const p = paneOf(id);
  if (p >= 0) {
    const free = (t) => paneOf(t) < 0;
    const next = state.tabs.slice(i).find(free) ?? state.tabs.slice(0, i).reverse().find(free) ?? null;
    if (next === null && state.panes.length > 1) removePane(p);
    else state.panes[p].run = next;
  }
  dropView(id);
  commit();
}

export function reopenTab() {
  while (closed.length) {
    const id = closed.pop();
    if (getRun(id)) { show(id); return true; }
  }
  return false;
}

/** Moves the focused pane to the tab `step` away (wrapping). */
export function cycleTab(step) {
  if (!state.tabs.length) return;
  const cur = state.tabs.indexOf(focusedRun());
  const n = state.tabs.length;
  const next = cur < 0 ? (step > 0 ? 0 : n - 1) : (cur + step + n) % n;
  show(state.tabs[next]);
}

/** Shows tab number `n` (1-based); 9 is the last, as in browsers. */
export function goToTab(n) {
  const id = n === 9 ? state.tabs[state.tabs.length - 1] : state.tabs[n - 1];
  if (id) show(id);
}

/**
 * A new pane to the right of the focused one, with the next tab that is
 * not on screen (or home). Refused when the panes would get too narrow.
 */
export function split(withNext = true) {
  if (narrow() || state.panes.length >= MAX_PANES) return false;
  if (area && area.clientWidth / (state.panes.length + 1) < MIN_PANE) return false;
  const cur = state.tabs.indexOf(focusedRun());
  const order = [...state.tabs.slice(cur + 1), ...state.tabs.slice(0, Math.max(cur, 0))];
  const next = withNext ? order.find((t) => paneOf(t) < 0) ?? null : null;
  state.panes.splice(state.focus + 1, 0, { run: next });
  state.sizes = Array(state.panes.length).fill(1 / state.panes.length);
  state.focus += 1;
  commit();
  return true;
}

/** Closes a pane (default: the focused one); its tab stays open. */
export function closePane(i = state.focus) {
  if (state.panes.length < 2) return;
  removePane(i);
  commit();
}

function removePane(i) {
  const run = state.panes[i].run;
  state.panes.splice(i, 1);
  state.sizes.splice(i, 1);
  state.sizes = fit(state.sizes, state.panes.length);
  state.focus = Math.min(i, state.panes.length - 1);
  if (run) views.get(run)?.deactivate();
}

export function focusPane(i) {
  if (i < 0 || i >= state.panes.length || i === state.focus) return;
  state.focus = i;
  commit();
  views.get(state.panes[i].run)?.focus();
}

/** Focus to the next pane (wrapping). */
export const otherPane = () => focusPane((state.focus + 1) % state.panes.length);

/** Replaces the panes with a page (the kit), or brings them back. */
export function showPage(node) {
  if (!node && !page) return;
  page?.destroy?.();
  page = node;
  if (node) { render(); return; }
  lastFocused = undefined; // the address goes back to the focused run
  commit();
}

// ---- rendering ------------------------------------------------------------

let onFocus = () => {};
/** `fn(runId | null)` hears when the focused pane's run changes. */
export const onFocusChange = (fn) => { onFocus = fn; };

let lastFocused;
function commit() {
  save();
  render();
  const run = focusedRun();
  if (run !== lastFocused) {
    lastFocused = run;
    // The address bar follows the focused pane without adding history.
    const hash = run ? `#run=${run}` : "";
    if (location.hash !== hash) history.replaceState(null, "", hash || location.pathname);
    setCurrent(run);
    onFocus(run);
  }
}

/** Puts the desk into the main area (again, after the sign-in form). */
export function mountDesk() {
  if (root) { if (!root.isConnected) { app.innerHTML = ""; app.appendChild(root); } return; }
  root = el(`<div class="workspace">
    <div class="tabbar" role="tablist" aria-label="Open runs"><div class="tabs"></div>
      <div class="bar-tools">
        <button class="icon-btn" type="button" data-split title="Split: another pane to the right (\\)" aria-label="Split pane">${ICON_SPLIT}</button>
        <button class="palette-btn" type="button" data-palette title="Search runs and commands"><span>Search</span><kbd>${MOD}K</kbd></button>
      </div></div>
    <div class="panes"></div></div>`);
  bar = root.querySelector(".tabs");
  area = root.querySelector(".panes");
  root.querySelector("[data-split]").addEventListener("click", () => { if (!split()) flashRefused(); });
  root.querySelector("[data-palette]").addEventListener("click", () => document.dispatchEvent(new CustomEvent("shiioo:palette")));
  wireTabs();
  app.innerHTML = "";
  app.appendChild(root);
  watch(() => { prune(); paintTabs(); });
  matchMedia("(max-width: 760px)").addEventListener("change", () => { for (const s of slots) clearSlot(s); render(); });
  let resizing = null;
  window.addEventListener("resize", () => { cancelAnimationFrame(resizing); resizing = requestAnimationFrame(render); });
  lastFocused = undefined;
  commit();
}

const ICON_SPLIT = `<svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true"><rect x="1.5" y="2.5" width="13" height="11" rx="2" fill="none" stroke="currentColor" stroke-width="1.3"/><path d="M8 2.5v11" stroke="currentColor" stroke-width="1.3"/></svg>`;

function flashRefused() {
  const b = root.querySelector("[data-split]");
  b.classList.add("refused");
  b.title = state.panes.length >= MAX_PANES ? "Three panes at most" : "Not enough room for another pane";
  setTimeout(() => { b.classList.remove("refused"); b.title = "Split: another pane to the right (\\)"; }, 1200);
}

/** Tabs whose run no longer exists close themselves. */
function prune() {
  if (!isLoaded()) return;
  const gone = state.tabs.filter((id) => !getRun(id));
  if (!gone.length) return;
  for (const id of gone) {
    state.tabs = state.tabs.filter((t) => t !== id);
    for (const p of state.panes) if (p.run === id) p.run = null;
    dropView(id);
  }
  commit();
}

function render() {
  if (!root) return;
  root.classList.toggle("paged", Boolean(page));
  let pageHost = root.querySelector(".page-host");
  if (page) {
    if (!pageHost) { pageHost = el(`<div class="page-host"></div>`); root.appendChild(pageHost); }
    if (page.root.parentNode !== pageHost) { pageHost.innerHTML = ""; pageHost.appendChild(page.root); }
    for (const s of slots) s.run && views.get(s.run)?.deactivate();
    return;
  }
  pageHost?.remove();
  paintTabs();
  const shown = visiblePanes();
  // One slot per shown pane, with a divider between neighbours.
  while (slots.length > shown.length) { const s = slots.pop(); clearSlot(s); s.el.remove(); s.divider?.remove(); }
  while (slots.length < shown.length) {
    const s = { el: el(`<section class="slot"></section>`), run: undefined, content: null };
    const i = slots.length;
    s.el.addEventListener("pointerdown", () => { if (s.pane !== state.focus) focusPane(s.pane); }, true);
    if (i > 0) { s.divider = el(`<div class="divider" role="separator" aria-orientation="vertical" title="Drag to resize, double-click to even out"></div>`); wireDivider(s.divider); area.appendChild(s.divider); }
    area.appendChild(s.el);
    slots.push(s);
  }
  // Detach first, then attach: a run moving between panes must leave its
  // old slot before it enters the new one.
  shown.forEach((p, k) => { if (slots[k].run !== state.panes[p].run) clearSlot(slots[k]); });
  shown.forEach((p, k) => {
    const s = slots[k];
    const run = state.panes[p].run;
    s.pane = p;
    s.el.style.flexGrow = shown.length > 1 ? String(state.sizes[p]) : "1";
    s.el.classList.toggle("focused", p === state.focus && shown.length > 1);
    s.el.setAttribute("aria-label", `Pane ${p + 1}`);
    if (s.content) { if (run) s.content.activate(); return; }
    s.run = run;
    if (run) {
      const v = viewFor(run);
      s.content = v;
      s.el.appendChild(v.root);
      v.activate();
    } else {
      s.content = createHome({ extra: p > 0 || state.tabs.length > 0 });
      s.el.appendChild(s.content.root);
    }
  });
  // Views no pane shows stop streaming.
  const onScreen = new Set(shown.map((p) => state.panes[p].run));
  for (const [id, v] of views) if (!onScreen.has(id)) v.deactivate();
}

function clearSlot(s) {
  if (!s.content) return;
  if (s.run) { s.content.deactivate(); s.content.root.remove(); } else s.content.destroy();
  s.content = null;
  s.run = undefined;
}

function paintTabs() {
  if (!bar) return;
  const focused = focusedRun();
  bar.innerHTML = state.tabs.map((id, i) => {
    const r = getRun(id);
    const st = r?.status.state;
    const where = paneOf(id);
    const cls = ["tab", id === focused ? "focused" : "", where >= 0 && id !== focused ? "shown" : "",
      st && needsYou(st) ? "attn" : "", r && unread(r) ? "unread" : ""].filter(Boolean).join(" ");
    const label = r ? (r.task || "(no task)") : "Loading…";
    return `<div class="${cls}" role="tab" aria-selected="${id === focused}" draggable="true" data-id="${esc(id)}" title="${esc(label)}${i < 9 ? ` (${i + 1})` : ""}">
      ${st ? mark(st) : `<span class="mk"></span>`}<span class="t">${esc(trunc(label, 48))}</span>
      <button class="x" type="button" tabindex="-1" aria-label="Close tab" title="Close tab (W)">×</button></div>`;
  }).join("") + (state.tabs.length ? "" : `<div class="tabs-empty">Runs you open appear here as tabs. Shift-click one to open it beside this pane.</div>`);
  bar.querySelector(".tab.focused")?.scrollIntoView({ block: "nearest", inline: "nearest" });
}

function wireTabs() {
  bar.addEventListener("click", (e) => {
    const tab = e.target.closest(".tab");
    if (!tab) return;
    if (e.target.closest(".x")) { closeTab(tab.dataset.id); return; }
    if (e.shiftKey) showBeside(tab.dataset.id); else show(tab.dataset.id);
  });
  bar.addEventListener("auxclick", (e) => { const tab = e.target.closest(".tab"); if (tab && e.button === 1) { e.preventDefault(); closeTab(tab.dataset.id); } });
  // Drag to reorder; drop on a pane to show it there.
  let dragged = null;
  bar.addEventListener("dragstart", (e) => { const t = e.target.closest(".tab"); if (!t) return; dragged = t.dataset.id; e.dataTransfer.effectAllowed = "move"; e.dataTransfer.setData("text/plain", dragged); t.classList.add("dragging"); });
  bar.addEventListener("dragend", () => { dragged = null; bar.querySelector(".dragging")?.classList.remove("dragging"); area.querySelectorAll(".drop").forEach((x) => x.classList.remove("drop")); });
  bar.addEventListener("dragover", (e) => {
    const t = e.target.closest(".tab");
    if (!dragged || !t || t.dataset.id === dragged) return;
    e.preventDefault();
    const from = state.tabs.indexOf(dragged);
    let to = state.tabs.indexOf(t.dataset.id);
    const box = t.getBoundingClientRect();
    if (e.clientX > box.left + box.width / 2 && to < from) to += 1;
    if (e.clientX < box.left + box.width / 2 && to > from) to -= 1;
    if (to === from) return;
    state.tabs.splice(to, 0, state.tabs.splice(from, 1)[0]);
    save();
    paintTabs();
    bar.querySelector(`[data-id="${CSS.escape(dragged)}"]`)?.classList.add("dragging");
  });
  area.addEventListener("dragover", (e) => {
    if (!dragged) return;
    const s = e.target.closest(".slot");
    if (!s) return;
    e.preventDefault();
    area.querySelectorAll(".drop").forEach((x) => x !== s && x.classList.remove("drop"));
    s.classList.add("drop");
  });
  area.addEventListener("dragleave", (e) => { if (e.target.classList?.contains("slot")) e.target.classList.remove("drop"); });
  area.addEventListener("drop", (e) => {
    const s = e.target.closest(".slot");
    if (!dragged || !s) return;
    e.preventDefault();
    s.classList.remove("drop");
    show(dragged, slots.find((x) => x.el === s).pane);
  });
}

function wireDivider(div) {
  div.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    const k = slots.findIndex((s) => s.divider === div); // the slot to the right
    const [a, b] = [slots[k - 1].pane, slots[k].pane];
    const left = slots[k - 1].el;
    const right = slots[k].el;
    const total = left.getBoundingClientRect().width + right.getBoundingClientRect().width;
    const share = state.sizes[a] + state.sizes[b];
    const start = e.clientX;
    const w0 = left.getBoundingClientRect().width;
    div.setPointerCapture(e.pointerId);
    div.classList.add("active");
    const move = (m) => {
      const w = Math.min(total - MIN_PANE * 0.75, Math.max(MIN_PANE * 0.75, w0 + m.clientX - start));
      state.sizes[a] = share * (w / total);
      state.sizes[b] = share - state.sizes[a];
      left.style.flexGrow = String(state.sizes[a]);
      right.style.flexGrow = String(state.sizes[b]);
    };
    const up = () => { div.classList.remove("active"); div.removeEventListener("pointermove", move); save(); };
    div.addEventListener("pointermove", move);
    div.addEventListener("pointerup", up, { once: true });
    div.addEventListener("pointercancel", up, { once: true });
  });
  div.addEventListener("dblclick", () => { state.sizes = Array(state.panes.length).fill(1 / state.panes.length); commit(); });
}

/**
 * The panes there is room for, always including the focused one. A window
 * made narrower hides panes instead of closing them; they come back when
 * it is wide again. A phone shows the focused pane only.
 */
function visiblePanes() {
  const all = state.panes.map((_, i) => i);
  if (narrow()) return [state.focus];
  const width = area?.clientWidth || 0;
  if (!width) return all;
  const fit = Math.max(1, Math.min(all.length, Math.floor(width / (MIN_PANE * 0.75))));
  const start = Math.min(Math.max(0, state.focus - fit + 1), all.length - fit);
  return all.slice(start, start + fit);
}

/** For the cheat sheet and the palette. */
export const limits = { MAX_PANES };
