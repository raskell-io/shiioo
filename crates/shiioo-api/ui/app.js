// Shiioo dashboard: a small hash-routed page over the /v1 API.
// Views: "" (run list), "#run=<id>" (one run with its live log).
(() => {
  "use strict";

  const app = document.getElementById("app");
  const TOKEN_KEY = "shiioo_token";
  let token = localStorage.getItem(TOKEN_KEY) || "";
  let stop = null; // closes the current view's background work

  // A token arriving in the fragment (#token=…) is stored and removed
  // from the URL so it never ends up in a screenshot or the history.
  const frag = new URLSearchParams(location.hash.replace(/^#/, ""));
  if (frag.get("token")) {
    token = frag.get("token");
    localStorage.setItem(TOKEN_KEY, token);
    history.replaceState(null, "", location.pathname);
  }

  const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
  const el = (html) => { const t = document.createElement("template"); t.innerHTML = html.trim(); return t.content.firstElementChild; };
  const short = (id) => String(id).slice(0, 8);
  const trunc = (s, n) => { s = String(s ?? "").replace(/\s+/g, " ").trim(); return s.length > n ? s.slice(0, n) + "…" : s; };

  function ago(ms) {
    const d = Math.max(0, Date.now() - ms) / 1000;
    if (d < 60) return "just now";
    if (d < 3600) return `${Math.floor(d / 60)} min ago`;
    if (d < 86400) return `${Math.floor(d / 3600)} h ago`;
    return `${Math.floor(d / 86400)} d ago`;
  }

  const GLYPH = { open: "▶", waiting_approval: "🔐", needs_attention: "⚠️", completed: "✅", failed: "❌", cancelled: "🛑" };

  // ---- API -------------------------------------------------------------
  class Unauthorized extends Error {}

  async function api(path, opts = {}) {
    const res = await fetch(path, { ...opts, headers: { Authorization: `Bearer ${token}`, "content-type": "application/json", ...(opts.headers || {}) } });
    if (res.status === 401) {
      localStorage.removeItem(TOKEN_KEY); token = "";
      renderToken("That token was not accepted.");
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

  // ---- routing ---------------------------------------------------------
  function route() {
    if (stop) { stop(); stop = null; }
    if (!token) return renderToken();
    const run = new URLSearchParams(location.hash.replace(/^#/, "")).get("run");
    if (run) renderRun(run); else renderList();
  }
  window.addEventListener("hashchange", route);
  document.getElementById("refresh").addEventListener("click", route);
  document.getElementById("new-task").addEventListener("click", () => { location.hash = ""; renderList(true); });

  // ---- token form ------------------------------------------------------
  function renderToken(message) {
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

  // ---- run list --------------------------------------------------------
  async function renderList(showForm = false) {
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
              <span class="muted">${esc(st.replace("_", " "))}${st === "waiting_approval" ? ": " + esc(r.status.summary) : ""}</span></span>
            <span class="meta">${esc(ago(r.updated_at))}<br><span class="id">${esc(short(r.id))}</span></span>
          </a></li>`));
        }
      } catch (e) { if (!(e instanceof Unauthorized)) showError(e); }
    };
    await load();
    const timer = setInterval(() => { if (document.visibilityState === "visible") load(); }, 10000);
    stop = () => clearInterval(timer);
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
      } catch (err) { if (!(err instanceof Unauthorized)) showError(err); }
    });
    return form;
  }

  function showError(e) {
    const box = el(`<div class="error">${esc(e.message || e)}</div>`);
    app.prepend(box);
    setTimeout(() => box.remove(), 6000);
  }

  // ---- one run ---------------------------------------------------------
  async function renderRun(id) {
    app.innerHTML = `<div class="run-head"><span class="muted">${esc(short(id))}</span><div class="task">…</div></div>`;
    const head = app.querySelector(".run-head");
    const approval = el(`<div></div>`);
    const actions = el(`<div class="btn-row"></div>`);
    const diffPanel = el(`<div></div>`);
    const log = el(`<div class="log"></div>`);
    app.append(approval, actions, diffPanel, el(`<h2>Log</h2>`), log);

    let current = null;
    const refresh = async () => {
      try { current = await api(`/v1/runs/${id}`); } catch (e) { if (!(e instanceof Unauthorized)) showError(e); return; }
      const st = current.status.state;
      head.innerHTML = `<span class="status-${esc(st)}">${GLYPH[st] || "•"} ${esc(st.replace("_", " "))}</span>
        <span class="muted"> · ${esc(short(id))}</span><div class="task">${esc(current.task || "")}</div>`;
      approval.innerHTML = "";
      if (st === "waiting_approval") {
        const card = el(`<div class="card approval"><strong>Approval needed</strong>
          <div class="summary">${esc(current.status.summary)}</div>
          <div class="btn-row"><button class="btn allow">Allow</button><button class="btn deny">Deny</button></div></div>`);
        for (const decision of ["allow", "deny"]) {
          card.querySelector(`.${decision}`).addEventListener("click", async (e) => {
            e.target.disabled = true;
            try { await api(`/v1/approvals/${current.status.approval_id}`, { method: "POST", body: JSON.stringify({ decision }) }); }
            catch (err) { if (!(err instanceof Unauthorized)) showError(err); }
            refresh();
          });
        }
        approval.appendChild(card);
      }
      actions.innerHTML = "";
      const open = st === "open" || st === "waiting_approval" || st === "needs_attention";
      if (open) {
        const cancel = el(`<button class="btn danger">Cancel</button>`);
        cancel.addEventListener("click", async () => {
          if (cancel.dataset.armed !== "1") { cancel.dataset.armed = "1"; cancel.textContent = "Really cancel?"; setTimeout(() => { cancel.dataset.armed = ""; cancel.textContent = "Cancel"; }, 4000); return; }
          cancel.disabled = true;
          try { await api(`/v1/runs/${id}/cancel`, { method: "POST", body: JSON.stringify({ reason: "from dashboard" }) }); }
          catch (err) { if (!(err instanceof Unauthorized)) showError(err); }
        });
        actions.appendChild(cancel);
        if (st === "open") {
          const resume = el(`<button class="btn">Resume</button>`);
          resume.addEventListener("click", async () => {
            resume.disabled = true;
            try { await api(`/v1/runs/${id}/resume`, { method: "POST" }); } catch (err) { if (!(err instanceof Unauthorized)) showError(err); }
          });
          actions.appendChild(resume);
        }
      }
      const diff = el(`<button class="btn">Diff</button>`);
      diff.addEventListener("click", async () => {
        diff.disabled = true;
        try {
          const text = await api(`/v1/runs/${id}/diff`);
          diffPanel.innerHTML = "";
          diffPanel.appendChild(el(`<pre class="diff">${esc(text || "(no changes on the branch)")}</pre>`));
        } catch (err) { if (!(err instanceof Unauthorized)) showError(err); }
        diff.disabled = false;
      });
      actions.appendChild(diff);
    };
    await refresh();

    // Live log over SSE. EventSource can't send the token, so the stream
    // is read with fetch and the frames are parsed here.
    let lastId = -1;
    let debounce = null;
    const onEvent = (ev) => {
      const atBottom = log.scrollHeight - log.scrollTop - log.clientHeight < 40;
      log.appendChild(el(`<div class="line kind-${esc(ev.payload.type)}"><span class="seq">#${ev.seq}</span><span>${esc(describe(ev))}</span></div>`));
      if (atBottom) log.scrollTop = log.scrollHeight;
      clearTimeout(debounce);
      debounce = setTimeout(refresh, 200);
    };
    const controller = new AbortController();
    stop = () => { controller.abort(); clearTimeout(debounce); };
    streamEvents(id, controller.signal, (frame) => {
      if (frame.id !== undefined) lastId = frame.id;
      if (frame.data) { try { onEvent(JSON.parse(frame.data)); } catch (_) { /* skip a bad frame */ } }
    }, () => lastId);
  }

  // Reads /events/stream, calling onFrame per SSE frame; reconnects with
  // Last-Event-ID until the run is finished or the view is left.
  async function streamEvents(id, signal, onFrame, lastId) {
    let backoff = 1000;
    while (!signal.aborted) {
      let finished = false;
      try {
        const headers = { Authorization: `Bearer ${token}` };
        if (lastId() >= 0) headers["Last-Event-ID"] = String(lastId());
        const res = await fetch(`/v1/runs/${id}/events/stream`, { headers, signal });
        if (res.status === 401) { localStorage.removeItem(TOKEN_KEY); token = ""; renderToken("That token was not accepted."); return; }
        if (!res.ok) throw new Error(`stream ${res.status}`);
        backoff = 1000;
        const reader = res.body.getReader();
        const decoder = new TextDecoder();
        let buf = "";
        for (;;) {
          const { value, done } = await reader.read();
          if (done) break;
          buf += decoder.decode(value, { stream: true }).replace(/\r\n/g, "\n");
          let cut;
          while ((cut = buf.indexOf("\n\n")) >= 0) {
            const frame = parseFrame(buf.slice(0, cut));
            buf = buf.slice(cut + 2);
            if (frame) { onFrame(frame); if (frame.terminal) finished = true; }
          }
        }
      } catch (e) {
        if (signal.aborted) return;
      }
      // The server closes the stream after the terminal event; a clean
      // close without one means a dropped connection.
      if (finished || signal.aborted) return;
      await new Promise((r) => setTimeout(r, backoff));
      backoff = Math.min(backoff * 2, 10000);
    }
  }

  function parseFrame(text) {
    const frame = { data: "" };
    const data = [];
    for (const line of text.split("\n")) {
      if (!line || line.startsWith(":")) continue; // keep-alive comments
      const i = line.indexOf(":");
      const field = i < 0 ? line : line.slice(0, i);
      const value = i < 0 ? "" : line.slice(i + 1).replace(/^ /, "");
      if (field === "id") frame.id = Number(value);
      else if (field === "event") frame.event = value;
      else if (field === "data") data.push(value);
    }
    if (!data.length && frame.id === undefined) return null;
    frame.data = data.join("\n");
    try {
      const t = JSON.parse(frame.data).payload?.type;
      frame.terminal = t === "run_completed" || t === "run_failed" || t === "run_cancelled";
    } catch (_) { /* not an event */ }
    return frame;
  }

  // One line per event, mirroring the server's describe().
  function describe(ev) {
    const p = ev.payload;
    const text = (content) => (Array.isArray(content) ? content : []).filter((b) => b.type === "text").map((b) => b.text).join(" ");
    switch (p.type) {
      case "run_created": return `run created: ${trunc(p.spec?.task, 100)}`;
      case "lease_acquired": return `lease acquired by ${p.worker} (epoch ${ev.epoch})`;
      case "model_response_recorded": return `${p.model}: ${trunc(text(p.response?.content), 140)}`;
      case "tool_intent": return `→ ${p.tool} ${trunc(JSON.stringify(p.input), 120)}`;
      case "tool_result": return `${p.is_error ? "✗" : "←"} ${trunc(typeof p.output === "string" ? p.output : JSON.stringify(p.output), 120)}`;
      case "tool_outcome_unknown": return `? outcome of ${p.call_id} unknown`;
      case "harness_session_started": return `${p.harness} session ${p.session_id} (${p.model})`;
      case "harness_result": return `harness result: ${p.subtype}, cost $${Number(p.cost_usd || 0).toFixed(4)}`;
      case "capacity_observed": return `capacity ${p.provider}: 5h ${Math.round((p.windows?.five_hour?.utilization || 0) * 100)}%, 7d ${Math.round((p.windows?.seven_day?.utilization || 0) * 100)}% used`;
      case "user_message": return `${p.author}: ${trunc(p.content, 140)}`;
      case "policy_decision": return `rules: ${p.decision} ${p.tool} (${p.reason})`;
      case "approval_requested": return `waiting for approval: ${p.summary}`;
      case "approval_decided": return `${p.decision} by ${p.by}`;
      case "cancel_requested": return `cancel requested by ${p.by}${p.reason ? ": " + p.reason : ""}`;
      case "sandbox_started": {
        const parts = [p.detail?.container, p.detail?.sandbox, p.detail?.egress?.mode ? `egress ${p.detail.egress.mode}` : null].filter(Boolean);
        return `isolation: ${p.level}${parts.length ? ` (${parts.join(", ")})` : ""}`;
      }
      case "secret_redacted": return `redacted secret ${p.name} (${p.count} occurrence(s))`;
      case "run_completed": return `completed: ${trunc(typeof p.output === "string" ? p.output : JSON.stringify(p.output), 140)}`;
      case "run_failed": return `failed: ${p.reason}`;
      case "run_cancelled": return `cancelled: ${p.reason}`;
      default: return p.type;
    }
  }

  route();
})();
