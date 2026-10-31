// The takeover terminal (U11): the harness's own UI, in a pane. xterm.js
// (vendored, loaded the first time a terminal opens) draws it with the
// desk's monospace font and colours; the office's WebSocket carries the
// bytes (ADR-0013). Each attach redeems a fresh single-use ticket, so a
// dropped connection rejoins the same process.

import { api, showError } from "../core.js";

let xterm = null; // the modules, once loaded
async function load() {
  if (!xterm) {
    if (!document.querySelector("link[data-xterm]")) {
      const css = document.createElement("link");
      css.rel = "stylesheet";
      css.href = "/ui/vendor/xterm.css";
      css.dataset.xterm = "";
      document.head.appendChild(css);
    }
    const [core, fit, webgl] = await Promise.all([
      import("../vendor/xterm.mjs"),
      import("../vendor/xterm-addon-fit.mjs"),
      import("../vendor/xterm-addon-webgl.mjs"),
    ]);
    xterm = { Terminal: core.Terminal, FitAddon: fit.FitAddon, WebglAddon: webgl.WebglAddon };
  }
  return xterm;
}

// ANSI colours per theme, tuned against the desk's panels; the rest of
// the theme comes from the tokens.
const ANSI = {
  light: ["#23262b", "#b33b3b", "#37734e", "#915b15", "#2f5f9e", "#8a3f8f", "#2c746b", "#d9d6cc",
    "#60656c", "#c94a4a", "#3f8a5c", "#a8691a", "#3a72b8", "#a04aa6", "#2f8a7f", "#f4f2ec"],
  dark: ["#12171a", "#e57373", "#6cc08b", "#e3a64c", "#79a8e8", "#c792ea", "#5eb4a6", "#c9c7c1",
    "#6b737b", "#ff8a8a", "#8ad8a6", "#f0bd6e", "#9cc0f2", "#d8aef2", "#7fd0c2", "#e8e6e0"],
};
const NAMES = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];

function themeFromTokens(el) {
  const cs = getComputedStyle(el);
  const v = (n) => cs.getPropertyValue(`--${n}`).trim();
  const dark = document.documentElement.dataset.theme === "dark";
  const ansi = ANSI[dark ? "dark" : "light"];
  const theme = {
    background: v("panel"), foreground: v("ink"), cursor: v("accent"), cursorAccent: v("panel"),
    selectionBackground: dark ? "#2a4f49" : "#cfe5e0", selectionInactiveBackground: dark ? "#223a36" : "#e1efec",
  };
  NAMES.forEach((n, i) => {
    theme[n] = ansi[i];
    theme[`bright${n[0].toUpperCase()}${n.slice(1)}`] = ansi[i + 8];
  });
  return theme;
}

/**
 * A terminal attached to run `id`'s takeover, inside `box`. Starts the
 * takeover if there is none. `onState(state, text)` hears `connecting`,
 * `live`, `reconnecting` and `ended` (with the reason).
 */
export async function openTerminal(box, id, { onState = () => {} } = {}) {
  onState("connecting", "Starting the terminal…");
  const { Terminal, FitAddon, WebglAddon } = await load();
  const term = new Terminal({
    fontFamily: '"JetBrains Mono", ui-monospace, Menlo, monospace',
    fontSize: 13,
    lineHeight: 1.2,
    cursorBlink: true,
    allowProposedApi: true,
    scrollback: 5000,
    macOptionIsMeta: true,
    theme: themeFromTokens(box),
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  term.open(box);
  // WebGL draws fastest; `localStorage.shiioo_term_renderer = "dom"` is
  // the way out on a GPU that draws it badly.
  if (localStorage.getItem("shiioo_term_renderer") !== "dom") {
    try {
      const gl = new WebglAddon();
      gl.onContextLoss(() => gl.dispose());
      term.loadAddon(gl);
    } catch (_) { /* the DOM renderer is fine */ }
  }
  // The font may still be loading; measure once it has.
  await document.fonts?.load('13px "JetBrains Mono"').catch(() => {});
  fit.fit();

  const enc = new TextEncoder();
  let ws = null;
  let ended = false;
  let disposed = false;
  let retry = 0;

  const send = (data) => { if (ws?.readyState === WebSocket.OPEN) ws.send(data); };
  const sendSize = () => send(JSON.stringify({ type: "resize", cols: term.cols, rows: term.rows }));
  term.onData((d) => send(enc.encode(d)));
  term.onBinary((d) => send(Uint8Array.from(d, (c) => c.charCodeAt(0))));
  term.onResize(() => sendSize());

  const connect = async () => {
    if (disposed || ended) return;
    let ticket;
    try {
      ticket = await api(`/v1/runs/${id}/takeover`, { method: "POST", body: JSON.stringify({ cols: term.cols, rows: term.rows }) });
    } catch (e) {
      showError(e);
      onState("ended", e.message || String(e));
      return;
    }
    const url = `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}${ticket.terminal}`;
    const sock = new WebSocket(url);
    sock.binaryType = "arraybuffer";
    ws = sock;
    let first = true;
    sock.onmessage = (m) => {
      if (typeof m.data === "string") {
        let msg = {};
        try { msg = JSON.parse(m.data); } catch (_) { return; }
        if (msg.type === "status") onState("connecting", msg.text);
        if (msg.type === "ended") { ended = true; onState("ended", msg.reason); }
        return;
      }
      if (first) {
        // A rejoin replays the recent screen: start from a clean one.
        if (retry > 0) term.reset();
        first = false;
        retry = 0;
        onState("live");
        sendSize();
      }
      term.write(new Uint8Array(m.data));
    };
    sock.onclose = () => {
      if (ws === sock) ws = null;
      if (disposed || ended) return;
      retry += 1;
      onState("reconnecting", "Reconnecting to the terminal…");
      setTimeout(connect, Math.min(1000 * retry, 5000));
    };
  };

  // Follow the pane's size, and the desk's theme.
  const resize = new ResizeObserver(() => { if (box.clientWidth && box.clientHeight) fit.fit(); });
  resize.observe(box);
  const retheme = () => { term.options.theme = themeFromTokens(box); };
  document.addEventListener("shiioo:theme", retheme);

  await connect();
  return {
    focus() { term.focus(); },
    get ended() { return ended; },
    dispose() {
      disposed = true;
      resize.disconnect();
      document.removeEventListener("shiioo:theme", retheme);
      ws?.close();
      term.dispose();
    },
  };
}
