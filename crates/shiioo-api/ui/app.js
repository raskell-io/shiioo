// Shiioo dashboard: a small hash-routed page over the /v1 API, written as
// native ES modules (no build step). Views: "" (run list), "#run=<id>"
// (one run with its live log).

import { hasToken, leave, renderToken, setRoute } from "./core.js";
import { renderList } from "./views/runs.js";
import { renderRun } from "./views/run.js";

function route() {
  leave();
  if (!hasToken()) return renderToken();
  const run = new URLSearchParams(location.hash.replace(/^#/, "")).get("run");
  if (run) renderRun(run); else renderList();
}

setRoute(route);
window.addEventListener("hashchange", route);
document.getElementById("refresh").addEventListener("click", route);
document.getElementById("new-task").addEventListener("click", () => { location.hash = ""; leave(); renderList(true); });
route();
