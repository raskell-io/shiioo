// A hand-rolled SSE client: EventSource can't send the token, so streams
// are read with fetch and the frames are parsed here.

import { authHeader, rejectToken } from "./core.js";

const TERMINAL = new Set(["run_completed", "run_failed", "run_cancelled"]);

/**
 * Reads the SSE stream at `path`, calling onFrame per frame; reconnects
 * with Last-Event-ID (from `lastId()`) until a terminal event arrives,
 * when `stopOnTerminal` is set, or `signal` aborts. `onDrop` hears about
 * each lost connection before the retry.
 */
export async function streamEvents(path, signal, onFrame, lastId, { stopOnTerminal = true, onDrop = () => {} } = {}) {
  let backoff = 1000;
  while (!signal.aborted) {
    let finished = false;
    try {
      const headers = authHeader();
      if (lastId() >= 0) headers["Last-Event-ID"] = String(lastId());
      const res = await fetch(path, { headers, signal });
      if (res.status === 401) { rejectToken(); return; }
      // No such run (a stale tab, a mistyped link): retrying won't help.
      if (res.status === 400 || res.status === 404) return;
      if (!res.ok) throw new Error(`stream ${res.status}`);
      backoff = 1000;
      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let buf = "";
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        buf += decoder.decode(value, { stream: true }).replace(/\r\n/g, "\n");
        let cut;
        while ((cut = buf.indexOf("\n\n")) >= 0) {
          const frame = parseFrame(buf.slice(0, cut));
          buf = buf.slice(cut + 2);
          if (frame) { onFrame(frame); if (stopOnTerminal && frame.terminal) finished = true; }
        }
      }
    } catch (e) {
      if (signal.aborted) return;
    }
    // A run's stream closes after its terminal event; a clean close
    // without one means a dropped connection.
    if (finished || signal.aborted) return;
    onDrop();
    await new Promise((r) => setTimeout(r, backoff));
    backoff = Math.min(backoff * 2, 10000);
  }
}

/**
 * `streamEvents` that lets go of its connection while the tab is hidden
 * (or the caller pauses it, as a pane does when another tab covers it) and
 * resumes from `lastId()` when it is shown again: browsers allow only six
 * connections per host over plain http, and every open desk tab would
 * otherwise hold several. Returns `{ stop, pause, resume }`.
 */
export function liveStream(path, onFrame, lastId, opts = {}) {
  let controller = null;
  let stopped = false;
  let paused = false;
  const start = () => {
    if (stopped || paused || controller || document.hidden) return;
    const mine = new AbortController();
    controller = mine;
    streamEvents(path, mine.signal, onFrame, lastId, opts).finally(() => { if (controller === mine) controller = null; });
  };
  const drop = () => { if (controller) { controller.abort(); controller = null; } };
  const onVisibility = () => (document.hidden ? drop() : start());
  document.addEventListener("visibilitychange", onVisibility);
  start();
  return {
    stop() { stopped = true; drop(); document.removeEventListener("visibilitychange", onVisibility); },
    pause() { paused = true; drop(); },
    resume() { paused = false; start(); },
  };
}

export function parseFrame(text) {
  const frame = { data: "" };
  const data = [];
  for (const line of text.split("\n")) {
    if (!line || line.startsWith(":")) continue; // keep-alive comments
    const i = line.indexOf(":");
    const field = i < 0 ? line : line.slice(0, i);
    const value = i < 0 ? "" : line.slice(i + 1).replace(/^ /, "");
    if (field === "id") frame.id = Number(value);
    else if (field === "event") frame.event = value;
    else if (field === "data") data.push(value);
  }
  if (!data.length && frame.id === undefined) return null;
  frame.data = data.join("\n");
  try {
    frame.terminal = TERMINAL.has(JSON.parse(frame.data).payload?.type);
  } catch (_) { /* not an event */ }
  return frame;
}
