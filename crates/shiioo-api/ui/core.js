// Shared by every view: the token, the API call, small DOM helpers, the
// routing hook, run states and the theme.

export const app = document.getElementById("app");
export const side = document.getElementById("side");
const TOKEN_KEY = "shiioo_token";
const THEME_KEY = "shiioo_theme";
let token = localStorage.getItem(TOKEN_KEY) || "";
let route = () => {};

// A token arriving in the fragment (#token=…) is stored and removed
// from the URL so it never ends up in a screenshot or the history.
const frag = new URLSearchParams(location.hash.replace(/^#/, ""));
if (frag.get("token")) {
  token = frag.get("token");
  localStorage.setItem(TOKEN_KEY, token);
  history.replaceState(null, "", location.pathname);
}

export const hasToken = () => Boolean(token);
export const authHeader = () => ({ Authorization: `Bearer ${token}` });
export const setRoute = (fn) => { route = fn; };
export const rerender = () => route();

export const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
export const el = (html) => { const t = document.createElement("template"); t.innerHTML = html.trim(); return t.content.firstElementChild; };
export const short = (id) => String(id).slice(0, 8);
export const trunc = (s, n) => { s = String(s ?? "").replace(/\s+/g, " ").trim(); return s.length > n ? s.slice(0, n) + "…" : s; };

/** Phone layout: one column, the list is the home page. */
export const narrow = () => matchMedia("(max-width: 760px)").matches;
export const isMac = /Mac|iPhone|iPad/.test(navigator.platform);
export const MOD = isMac ? "⌘" : "Ctrl";

/** "now", "4m", "3h", "2d": the compact age the lists use. */
export function age(ms) {
  const d = Math.max(0, Date.now() - ms) / 1000;
  if (d < 45) return "now";
  if (d < 3600) return `${Math.round(d / 60)}m`;
  if (d < 86400) return `${Math.round(d / 3600)}h`;
  return `${Math.round(d / 86400)}d`;
}

/** "just now", "14 min ago": the age a log line shows. */
export function ago(ms) {
  const d = Math.max(0, Date.now() - ms) / 1000;
  if (d < 45) return "just now";
  if (d < 3600) return `${Math.round(d / 60)} min ago`;
  if (d < 86400) return `${Math.round(d / 3600)} h ago`;
  return `${Math.round(d / 86400)} d ago`;
}

/** A duration: "0.4s", "41.2s", "3 min", "2 h 5 min". */
export function span(ms) {
  if (ms < 60000) return `${(ms / 1000).toFixed(ms < 10000 ? 1 : 0)}s`;
  const m = Math.round(ms / 60000);
  return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${m % 60} min`;
}

export const exact = (ms) => new Date(ms).toLocaleString();

// ---- run states --------------------------------------------------------
// Only "needs you" carries the attention colour; working runs get a calm
// mark, finished ones fade.

/** The state as the mark and pill show it. */
export function kind(st) {
  switch (st) {
    case "open": case "taken_over": return "working";
    case "waiting_approval": case "needs_attention": case "parked": return "waiting";
    case "completed": return "done";
    case "failed": return "failed";
    case "cancelled": return "cancelled";
    default: return "done";
  }
}
export const needsYou = (st) => kind(st) === "waiting";
export const isOpen = (st) => st === "open" || st === "taken_over" || needsYou(st);

const LABEL = {
  open: "Working",
  waiting_approval: "Needs your approval",
  needs_attention: "Needs attention",
  parked: "Waiting for you",
  taken_over: "At the keyboard",
  completed: "Done",
  failed: "Failed",
  cancelled: "Cancelled",
};
export const stateLabel = (st) => LABEL[st] || String(st).replace("_", " ");
export const mark = (st) => `<span class="mk ${kind(st)}" aria-hidden="true"></span>`;

const WHO = { "claude-code": "Claude Code", codex: "Codex" };
/** Who works on a run, as people say it. */
export const who = (runtime) => WHO[runtime] || runtime || "Agent";

// ---- theme -------------------------------------------------------------
export const theme = () => localStorage.getItem(THEME_KEY) || "system";
const systemDark = matchMedia("(prefers-color-scheme: dark)");
/** Puts the chosen theme (or the system's) on <html>. */
function applyTheme() {
  const t = theme();
  document.documentElement.dataset.theme = t === "system" ? (systemDark.matches ? "dark" : "light") : t;
}
systemDark.addEventListener("change", () => { if (theme() === "system") applyTheme(); });
export function setTheme(t) {
  if (t === "system") localStorage.removeItem(THEME_KEY); else localStorage.setItem(THEME_KEY, t);
  applyTheme();
  document.dispatchEvent(new CustomEvent("shiioo:theme"));
}

// ---- API ---------------------------------------------------------------
export class Unauthorized extends Error {}

/** Forgets the token and asks for a new one. */
export function rejectToken() {
  localStorage.removeItem(TOKEN_KEY); token = "";
  renderToken("That token was not accepted.");
}

export async function api(path, opts = {}) {
  const res = await fetch(path, { ...opts, headers: { ...authHeader(), "content-type": "application/json", ...(opts.headers || {}) } });
  if (res.status === 401) {
    rejectToken();
    throw new Unauthorized();
  }
  if (!res.ok) {
    let detail = res.statusText;
    try { const p = await res.json(); detail = p.detail || p.title || detail; } catch (_) { /* not json */ }
    throw new Error(detail);
  }
  const type = res.headers.get("content-type") || "";
  return type.includes("json") ? res.json() : res.text();
}

/** A short message at the bottom of the screen. */
export function toast(text, { bad = false } = {}) {
  const box = el(`<div class="toast${bad ? " bad" : ""}" role="status">${esc(text)}</div>`);
  document.getElementById("toasts").appendChild(box);
  setTimeout(() => { box.classList.add("out"); setTimeout(() => box.remove(), 300); }, bad ? 6000 : 2600);
}

export function showError(e) {
  if (e instanceof Unauthorized) return;
  toast(e.message || String(e), { bad: true });
}

// ---- token form --------------------------------------------------------
export function renderToken(message) {
  side.hidden = true;
  app.innerHTML = "";
  const form = el(`<form class="empty sign-in">
    <div class="art" aria-hidden="true"><span class="brand-mark">s</span></div>
    <h2>${message ? esc(message) : "This office is private"}</h2>
    <p>Paste the API token from <code>.shiioo/api-token</code> on the office machine, or open the link <code>shiioo up</code> prints.</p>
    <div class="field"><label for="tok">API token</label>
      <input id="tok" type="password" autocomplete="off" autocapitalize="off" spellcheck="false"></div>
    <button class="btn primary" type="submit">Open the office</button>
  </form>`);
  form.addEventListener("submit", (e) => {
    e.preventDefault();
    token = form.querySelector("#tok").value.trim();
    if (token) { localStorage.setItem(TOKEN_KEY, token); side.hidden = false; route(); }
  });
  app.appendChild(form);
  form.querySelector("#tok").focus();
}
