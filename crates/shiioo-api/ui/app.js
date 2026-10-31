// The desk: a no-build dashboard served by the office, split into plain
// native ES modules. The address says what the focused pane shows: ""
// (home), "#run=<id>" (a run, opened as a tab), "#kit" (every component in
// both themes). Tabs, panes and their sizes live in `desk.js`.

import { hasToken, renderToken, setRoute, side } from "./core.js";
import * as desk from "./desk.js";
import { installKeys } from "./keys.js";
import { startOffice } from "./office.js";
import { createKit } from "./views/kit.js";
import { openNewTask } from "./views/runs.js";
import { renderSide } from "./views/side.js";

let started = false;
let paged = false;

function route() {
  if (!hasToken()) { renderToken(); return; }
  side.hidden = false;
  const first = !started;
  if (!started) {
    started = true;
    startOffice();
    renderSide();
    installKeys();
  }
  desk.mountDesk();
  const params = new URLSearchParams(location.hash.replace(/^#/, ""));
  if (params.has("kit")) { paged = true; desk.showPage(createKit()); return; }
  const fromPage = paged;
  if (paged) { paged = false; desk.showPage(null); }
  const run = params.get("run");
  // A bare address on load (or back from a page) keeps the saved layout;
  // later it means home in the focused pane.
  if (run) desk.show(run);
  else if (!first && !fromPage) desk.show(null);
  if (params.has("new")) { history.replaceState(null, "", location.pathname); openNewTask(); }
}

setRoute(route);
window.addEventListener("hashchange", route);
route();
