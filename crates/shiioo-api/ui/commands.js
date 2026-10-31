// Everything the desk can do from the keyboard, in one list: the keys, the
// palette and the cheat sheet all read it, so a command never has a key
// the palette doesn't show or a palette entry the sheet forgets.
//
// A command: { id, title (string or fn), group, keys: [e.key values],
// label (how the key is shown), when (fn: available now?), run (fn),
// confirm (palette asks for a second Enter), desk (desk layout only) }.

import { isOpen, MOD, needsYou, setTheme, theme, who } from "./core.js";
import * as desk from "./desk.js";
import { allRuns, getRun } from "./office.js";
import { openNewTask } from "./views/runs.js";

const view = () => desk.focusedView();
const run = () => view()?.view || null;
const pending = () => view()?.pending() || null;
const harnessRun = () => ["claude-code", "codex"].includes(run()?.runtime);

/** Cycles through what needs you, oldest first. */
export function nextNeedsYou() {
  const attn = allRuns().filter((r) => needsYou(r.status.state)).reverse();
  if (!attn.length) return false;
  const i = attn.findIndex((r) => r.id === desk.focusedRun());
  desk.show(attn[(i + 1) % attn.length].id);
  return true;
}

export const GROUPS = ["Office", "This run", "Tabs and panes", "Look"];

export const COMMANDS = [
  // Office
  { id: "new", group: "Office", title: "New task", keys: ["n"], label: "N", run: () => openNewTask() },
  { id: "new-here", group: "Office", title: () => `New task in ${run()?.repo}`, when: () => Boolean(run()?.repo), run: () => openNewTask({ repo: run().repo }) },
  { id: "next", group: "Office", title: "Next run that needs you", keys: ["j"], label: "J",
    when: () => allRuns().some((r) => needsYou(r.status.state)), run: nextNeedsYou },
  { id: "home", group: "Office", title: "Home", keys: ["h"], label: "H", run: () => desk.show(null) },
  { id: "palette", group: "Office", title: "Search runs and commands", label: `${MOD} K`, hidden: true },
  { id: "help", group: "Office", title: "Keyboard shortcuts", keys: ["?"], label: "?", run: () => document.dispatchEvent(new CustomEvent("shiioo:help")) },

  // The run in the focused pane
  { id: "allow", group: "This run", title: () => `Allow: ${pending()?.summary || ""}`, when: () => Boolean(pending()), run: () => view().answer("allow"), attention: true },
  { id: "deny", group: "This run", title: () => `Deny: ${pending()?.summary || ""}`, when: () => Boolean(pending()), run: () => view().answer("deny"), attention: true },
  { id: "approval", group: "This run", title: "Go to the approval", keys: ["a"], label: "A", when: () => Boolean(pending()), run: () => view().focusApproval() },
  { id: "reply", group: "This run", keys: ["r"], label: "R", help: "Write to this run, reply or follow up",
    title: () => (run() && !isOpen(run().status.state) ? "Follow up on this run" : run()?.status.state === "parked" ? "Reply" : "Write to this run"),
    when: () => Boolean(run()) && (isOpen(run().status.state) || harnessRun()), run: () => view().focusReply() },
  { id: "aside", group: "This run", title: "Approval and diff panel", keys: ["d"], label: "D", when: () => Boolean(view()), run: () => view().toggleAside() },
  { id: "raw", group: "This run", title: "Raw log (every event)", keys: ["l"], label: "L", when: () => Boolean(view()), run: () => view().toggleRaw() },
  { id: "take-over", group: "This run", title: "Take over: sit at the agent's terminal", keys: ["t"], label: "T",
    when: () => Boolean(view()?.canTakeOver()), run: () => view().takeOver() },
  { id: "join", group: "This run", title: "Join the terminal (taken over elsewhere)",
    when: () => run()?.status.state === "taken_over" && !view().hasTerminal(), run: () => view().takeOver() },
  { id: "hand-back", group: "This run", title: "Hand back (it waits for your next message)", when: () => run()?.status.state === "taken_over", run: () => view().handBack(false) },
  { id: "hand-back-carry", group: "This run", title: "Hand back and let it carry on", when: () => run()?.status.state === "taken_over", run: () => view().handBack(true) },
  { id: "resume", group: "This run", title: "Resume (start a worker if none is on it)", when: () => run()?.status.state === "open", run: () => view().resume() },
  { id: "cancel", group: "This run", title: () => `Cancel: ${run()?.task || ""}`, when: () => Boolean(run()) && isOpen(run().status.state),
    run: () => view().cancel(), confirm: "Enter again to cancel the run" },
  { id: "copy-id", group: "This run", title: "Copy the run id", when: () => Boolean(run()), run: () => navigator.clipboard?.writeText(run().id) },
  { id: "origin", group: "This run", title: "Open the run this follows up", when: () => Boolean(run()?.origin && getRun(run().origin)), run: () => desk.show(run().origin) },

  // Tabs and panes
  { id: "tab-next", group: "Tabs and panes", title: "Next tab", keys: ["]"], label: "]", desk: true, when: () => desk.tabs().length > 1, run: () => desk.cycleTab(1) },
  { id: "tab-prev", group: "Tabs and panes", title: "Previous tab", keys: ["["], label: "[", desk: true, when: () => desk.tabs().length > 1, run: () => desk.cycleTab(-1) },
  { id: "tab-n", group: "Tabs and panes", title: "Go to tab 1 to 8; 9 is the last", keys: ["1", "2", "3", "4", "5", "6", "7", "8", "9"], label: "1–9", desk: true, hidden: true,
    run: (e) => desk.goToTab(Number(e.key)) },
  { id: "tab-close", group: "Tabs and panes", title: "Close tab", keys: ["w"], label: "W", desk: true, when: () => Boolean(desk.focusedRun()) || desk.panes() > 1, run: () => desk.closeTab() },
  { id: "tab-reopen", group: "Tabs and panes", title: "Reopen the closed tab", keys: ["T"], label: "⇧ T", desk: true, run: () => desk.reopenTab() },
  { id: "split", group: "Tabs and panes", title: "Split: another pane to the right", keys: ["\\"], label: "\\", desk: true, when: () => desk.panes() < desk.limits.MAX_PANES, run: () => desk.split() },
  { id: "pane-other", group: "Tabs and panes", title: "Focus the next pane", keys: ["o"], label: "O", desk: true, when: () => desk.panes() > 1, run: () => desk.otherPane() },
  { id: "pane-close", group: "Tabs and panes", title: "Close this pane (its tab stays open)", keys: ["|"], label: "⇧ \\", desk: true, when: () => desk.panes() > 1, run: () => desk.closePane() },

  // Look
  { id: "theme-system", group: "Look", title: "Theme: follow the system", when: () => theme() !== "system", run: () => setTheme("system") },
  { id: "theme-light", group: "Look", title: "Theme: light", when: () => theme() !== "light", run: () => setTheme("light") },
  { id: "theme-dark", group: "Look", title: "Theme: dark", when: () => theme() !== "dark", run: () => setTheme("dark") },
  { id: "kit", group: "Look", title: "Component kit (every component, both themes)", run: () => { location.hash = "kit"; } },
];

export const titleOf = (c) => (typeof c.title === "function" ? c.title() : c.title);
export const available = (c) => !c.when || c.when();

/** "Claude Code · sango": a run's second line in lists. */
export const runLine = (r) => [r.repo, who(r.runtime)].filter(Boolean).join(" · ");
