// The desk: a no-build dashboard served by the office, split into plain
// native ES modules. Routes: "" (home: the run list on a phone, a quiet
// overview on a desk) and "#run=<id>". The sidebar lives across routes.

import { app, hasToken, leave, renderToken, setRoute, side } from "./core.js";
import { startOffice } from "./office.js";
import { renderSide, setCurrent } from "./views/side.js";
import { renderHome, openNewTask } from "./views/runs.js";
import { renderRun } from "./views/run.js";
import { installKeys } from "./keys.js";

let started = false;

function route() {
  leave();
  if (!hasToken()) { renderToken(); return; }
  side.hidden = false;
  if (!started) {
    started = true;
    startOffice();
    renderSide();
    installKeys();
  }
  const params = new URLSearchParams(location.hash.replace(/^#/, ""));
  const run = params.get("run");
  setCurrent(run);
  app.scrollTop = 0;
  if (run) renderRun(run);
  else renderHome();
  if (params.has("new")) { history.replaceState(null, "", location.pathname); openNewTask(); }
}

setRoute(route);
window.addEventListener("hashchange", route);
route();
