// The office as the desk knows it: every run's view, kept current by the
// office-wide stream (`/v1/events/stream`), and the providers' capacity.
// Views subscribe and re-render from here instead of polling.

import { api, showError } from "./core.js";
import { streamEvents } from "./stream.js";

const runs = new Map(); // id -> RunView
let capacity = [];
let live = false;
const listeners = new Set();

/** Calls `fn` now and after every change; returns the unsubscribe. */
export function watch(fn) {
  listeners.add(fn);
  fn();
  return () => listeners.delete(fn);
}

const changed = () => { for (const fn of listeners) fn(); };

/** All runs, most recently updated first. */
export const allRuns = () => [...runs.values()].sort((a, b) => b.updated_at - a.updated_at);
export const getRun = (id) => runs.get(id);
export const getCapacity = () => capacity;
export const isLive = () => live;

/** A view the desk got from elsewhere (a POST's answer), shown at once. */
export function upsert(view) {
  runs.set(view.id, view);
  changed();
}

export async function startOffice() {
  try {
    for (const r of await api("/v1/runs")) runs.set(r.id, r);
    changed();
  } catch (e) { showError(e); }
  loadCapacity();
  setInterval(() => { if (document.visibilityState === "visible") loadCapacity(); }, 60000);

  // Resume from the newest change we have; the office sends what moved
  // since, then a `ready` frame, then each change as it happens.
  let newest = Math.max(0, ...[...runs.values()].map((r) => r.updated_at));
  let batch = null;
  const flush = () => { batch = null; changed(); };
  streamEvents("/v1/events/stream", new AbortController().signal, (frame) => {
    if (!live) { live = true; changed(); }
    if (frame.id !== undefined) newest = Math.max(newest, frame.id);
    if (frame.event !== "run" || !frame.data) return;
    try {
      const view = JSON.parse(frame.data);
      runs.set(view.id, view);
      // A burst of frames re-renders once.
      if (!batch) batch = requestAnimationFrame(flush);
    } catch (_) { /* skip a bad frame */ }
  }, () => newest, { stopOnTerminal: false, onDrop: () => { live = false; changed(); } });
}

async function loadCapacity() {
  try { capacity = await api("/v1/capacity"); changed(); } catch (_) { /* not essential */ }
}
