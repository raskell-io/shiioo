// Keys for the whole desk, from the command list (`commands.js`). Single
// keys only act when no text field has the focus and no sheet is open;
// ⌘K / Ctrl-K opens the palette from anywhere, Esc closes the nearest
// thing: the palette or a sheet (they handle it), else the side panel.

import { available, COMMANDS } from "./commands.js";
import { narrow } from "./core.js";
import * as desk from "./desk.js";
import { openHelp, openPalette } from "./palette.js";

const typing = (t) => t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));
const overlay = () => document.querySelector(".scrim");

export function installKeys() {
  document.addEventListener("shiioo:palette", () => openPalette());
  document.addEventListener("shiioo:help", () => openHelp());
  document.addEventListener("keydown", (e) => {
    if (e.defaultPrevented) return;
    if ((e.metaKey || e.ctrlKey) && !e.altKey && e.key.toLowerCase() === "k") {
      e.preventDefault();
      if (!overlay()) openPalette();
      return;
    }
    if (overlay()) return;
    // In the agent's terminal every key is the agent's (Esc interrupts it).
    if (e.target instanceof Element && e.target.closest(".xterm")) return;
    if (e.key === "Escape") {
      if (typing(e.target)) { e.target.blur(); desk.focusedView()?.focus(); return; }
      if (desk.focusedView()?.escape()) e.preventDefault();
      return;
    }
    if (e.metaKey || e.ctrlKey || e.altKey || typing(e.target)) return;
    // Upper case keys mean Shift (⇧T); letters otherwise match either case.
    const cmd = COMMANDS.find((c) => c.keys?.some((k) => (k.length === 1 && /[a-z]/.test(k) ? e.key === k || (e.key === k.toUpperCase() && !e.shiftKey) : e.key === k)));
    if (!cmd || !cmd.run || (cmd.desk && narrow()) || !available(cmd)) return;
    e.preventDefault();
    cmd.run(e);
  });
}
