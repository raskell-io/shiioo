// Keys for the whole desk. Single letters only act when no text field has
// the focus: N new task, J the next run that needs you, R reply (the
// message box), D the diff panel. Views add their own with `onKey`.

import { needsYou } from "./core.js";
import { allRuns } from "./office.js";
import { openNewTask } from "./views/runs.js";

const typing = (t) => t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));

export function installKeys() {
  document.addEventListener("keydown", (e) => {
    if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey || typing(e.target) || document.querySelector(".scrim")) return;
    switch (e.key.toLowerCase()) {
      case "n": e.preventDefault(); openNewTask(); break;
      case "j": {
        // Cycles through what needs you, oldest first.
        const attn = allRuns().filter((r) => needsYou(r.status.state)).reverse();
        if (!attn.length) break;
        const current = new URLSearchParams(location.hash.replace(/^#/, "")).get("run");
        const i = attn.findIndex((r) => r.id === current);
        e.preventDefault();
        location.hash = `run=${attn[(i + 1) % attn.length].id}`;
        break;
      }
      case "r": { const box = document.querySelector(".composer textarea"); if (box && !box.closest("[hidden]")) { e.preventDefault(); box.focus(); } break; }
      case "d": document.querySelector("[data-aside-toggle]")?.click(); break;
      default:
    }
  });
}
