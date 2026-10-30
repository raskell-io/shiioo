// The run list: capacity line, one row per run, the new-task form.

import { api, app, ago, el, esc, GLYPH, onLeave, short, showError, stateLabel } from "../core.js";

export async function renderList(showForm = false) {
  app.innerHTML = "";
  if (showForm) app.appendChild(newTaskForm());
  const capacity = el(`<div class="muted capacity"></div>`);
  app.appendChild(capacity);
  const list = el(`<ul class="list"></ul>`);
  app.appendChild(list);
  const load = async () => {
    try {
      // Subscription windows per provider, e.g. "claude-code · 5h 19% · 7d 5%".
      api("/v1/capacity").then((providers) => {
        capacity.textContent = providers.map((p) => [p.provider, ...Object.entries(p.windows).map(([name, w]) =>
          `${{ five_hour: "5h", seven_day: "7d" }[name] || name} ${Math.round(w.utilization * 100)}%`)].join(" · ")).join("   ");
      }).catch(() => {});
      const runs = await api("/v1/runs");
      list.innerHTML = runs.length ? "" : `<li class="empty">No runs yet. Start one with “New task”.</li>`;
      for (const r of runs) {
        const st = r.status.state;
        list.appendChild(el(`<li><a class="row" href="#run=${esc(r.id)}">
          <span class="glyph status-${esc(st)}">${GLYPH[st] || "•"}</span>
          <span><span class="task">${esc(r.task || "(no task)")}</span>
            <span class="muted">${esc(stateLabel(st))}${st === "waiting_approval" ? ": " + esc(r.status.summary) : ""}</span></span>
          <span class="meta">${esc(ago(r.updated_at))}<br><span class="id">${esc(short(r.id))}</span></span>
        </a></li>`));
      }
    } catch (e) { showError(e); }
  };
  await load();
  const timer = setInterval(() => { if (document.visibilityState === "visible") load(); }, 10000);
  onLeave(() => clearInterval(timer));
}

function newTaskForm() {
  const form = el(`<form class="sheet">
    <label for="task">Task</label>
    <textarea id="task" required placeholder="Fix the flaky test in tests/net.rs and commit"></textarea>
    <label for="repo">repo path (optional)</label>
    <input id="repo" placeholder="/Users/me/code/project" autocapitalize="off" autocorrect="off">
    <div class="btn-row"><button class="btn primary" type="submit">Start</button>
      <button class="btn" type="button" data-cancel>Close</button></div>
  </form>`);
  form.querySelector("[data-cancel]").addEventListener("click", () => form.remove());
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    const task = form.querySelector("#task").value.trim();
    const repo = form.querySelector("#repo").value.trim();
    // The server's capacity broker picks the employee.
    const body = { task, employee: "auto", ...(repo ? { repo } : {}) };
    try {
      const run = await api("/v1/runs", { method: "POST", body: JSON.stringify(body) });
      location.hash = `run=${run.id}`;
    } catch (err) { showError(err); }
  });
  return form;
}
