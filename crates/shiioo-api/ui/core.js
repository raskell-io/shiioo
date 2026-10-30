// Shared by every view: the token, the API call, small DOM helpers, the
// current view's teardown and the routing hook.

export const app = document.getElementById("app");
const TOKEN_KEY = "shiioo_token";
let token = localStorage.getItem(TOKEN_KEY) || "";
let stop = null; // closes the current view's background work
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

/** Registers what to stop when the view is left. */
export const onLeave = (fn) => { stop = fn; };
/** Stops the current view's background work, if any. */
export function leave() { if (stop) { stop(); stop = null; } }

export const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
export const el = (html) => { const t = document.createElement("template"); t.innerHTML = html.trim(); return t.content.firstElementChild; };
export const short = (id) => String(id).slice(0, 8);
export const trunc = (s, n) => { s = String(s ?? "").replace(/\s+/g, " ").trim(); return s.length > n ? s.slice(0, n) + "…" : s; };

export function ago(ms) {
  const d = Math.max(0, Date.now() - ms) / 1000;
  if (d < 60) return "just now";
  if (d < 3600) return `${Math.floor(d / 60)} min ago`;
  if (d < 86400) return `${Math.floor(d / 3600)} h ago`;
  return `${Math.floor(d / 86400)} d ago`;
}

export const GLYPH = { open: "▶", waiting_approval: "🔐", needs_attention: "⚠️", completed: "✅", failed: "❌", cancelled: "🛑" };

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

export function showError(e) {
  if (e instanceof Unauthorized) return;
  const box = el(`<div class="error">${esc(e.message || e)}</div>`);
  app.prepend(box);
  setTimeout(() => box.remove(), 6000);
}

// ---- token form --------------------------------------------------------
export function renderToken(message) {
  app.innerHTML = "";
  const form = el(`<form class="sheet">
    <p>${message ? esc(message) : "This office is private."}</p>
    <label for="tok">Paste the API token from <code>.shiioo/api-token</code></label>
    <input id="tok" type="password" autocomplete="off" autocapitalize="off" spellcheck="false">
    <div class="btn-row"><button class="btn primary" type="submit">Open</button></div>
  </form>`);
  form.addEventListener("submit", (e) => {
    e.preventDefault();
    token = form.querySelector("#tok").value.trim();
    if (token) { localStorage.setItem(TOKEN_KEY, token); route(); }
  });
  app.appendChild(form);
}
