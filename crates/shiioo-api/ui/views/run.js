// One run: header, approval card, actions, diff, and the live log.

import { api, app, el, esc, GLYPH, onLeave, short, showError, stateLabel } from "../core.js";
import { describe } from "../describe.js";
import { streamEvents } from "../stream.js";

export async function renderRun(id) {
  app.innerHTML = `<div class="run-head"><span class="muted">${esc(short(id))}</span><div class="task">…</div></div>`;
  const head = app.querySelector(".run-head");
  const approval = el(`<div></div>`);
  const actions = el(`<div class="btn-row"></div>`);
  const diffPanel = el(`<div></div>`);
  const log = el(`<div class="log"></div>`);
  const composer = messageBox(id);
  app.append(approval, actions, diffPanel, el(`<h2>Log</h2>`), log, composer.form);

  let current = null;
  let harness = null; // "claude-code" or "codex", from the run's spec
  const refresh = async () => {
    try { current = await api(`/v1/runs/${id}`); } catch (e) { showError(e); return; }
    const st = current.status.state;
    composer.update(st, harness);
    head.innerHTML = `<span class="status-${esc(st)}">${GLYPH[st] || "•"} ${esc(stateLabel(st))}</span>
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
          catch (err) { showError(err); }
          refresh();
        });
      }
      approval.appendChild(card);
    }
    actions.innerHTML = "";
    const open = st === "open" || st === "waiting_approval" || st === "needs_attention" || st === "parked";
    if (open) {
      const cancel = el(`<button class="btn danger">Cancel</button>`);
      cancel.addEventListener("click", async () => {
        if (cancel.dataset.armed !== "1") { cancel.dataset.armed = "1"; cancel.textContent = "Really cancel?"; setTimeout(() => { cancel.dataset.armed = ""; cancel.textContent = "Cancel"; }, 4000); return; }
        cancel.disabled = true;
        try { await api(`/v1/runs/${id}/cancel`, { method: "POST", body: JSON.stringify({ reason: "from dashboard" }) }); }
        catch (err) { showError(err); }
      });
      actions.appendChild(cancel);
      if (st === "open") {
        const resume = el(`<button class="btn">Resume</button>`);
        resume.addEventListener("click", async () => {
          resume.disabled = true;
          try { await api(`/v1/runs/${id}/resume`, { method: "POST" }); } catch (err) { showError(err); }
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
      } catch (err) { showError(err); }
      diff.disabled = false;
    });
    actions.appendChild(diff);
  };
  await refresh();

  let lastId = -1;
  let debounce = null;
  const onEvent = (ev) => {
    if (ev.payload.type === "run_created") harness = ev.payload.spec?.runtime?.harness || null;
    const atBottom = log.scrollHeight - log.scrollTop - log.clientHeight < 40;
    log.appendChild(el(`<div class="line kind-${esc(ev.payload.type)}"><span class="seq">#${ev.seq}</span><span>${esc(describe(ev))}</span></div>`));
    if (atBottom) log.scrollTop = log.scrollHeight;
    clearTimeout(debounce);
    debounce = setTimeout(refresh, 200);
  };
  const controller = new AbortController();
  onLeave(() => { controller.abort(); clearTimeout(debounce); });
  streamEvents(`/v1/runs/${id}/events/stream`, controller.signal, (frame) => {
    if (frame.id !== undefined) lastId = frame.id;
    if (frame.data) { try { onEvent(JSON.parse(frame.data)); } catch (_) { /* skip a bad frame */ } }
  }, () => lastId);
}

// Writing to the run. A message goes in at the run's next turn: a native
// run reads it before its next model call, a harness when its current
// process finishes, a parked run wakes for it. Shown while the run can
// still read it.
function messageBox(id) {
  const form = el(`<form class="composer" hidden>
    <label class="hint muted" for="message"></label>
    <div class="composer-row">
      <textarea id="message" rows="1" placeholder="Message" enterkeyhint="send"></textarea>
      <button class="btn primary" type="submit" aria-label="Send">Send</button>
    </div>
  </form>`);
  const text = form.querySelector("textarea");
  const send = form.querySelector("button");
  const hint = form.querySelector(".hint");
  let base = "";
  // One key per message, kept until it is accepted, so a retry after a
  // dropped connection can't deliver it twice.
  let key = null;

  const grow = () => { text.style.height = "auto"; text.style.height = `${Math.min(text.scrollHeight, 200)}px`; };
  text.addEventListener("input", () => { if (!text.value.trim()) key = null; grow(); });
  text.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); form.requestSubmit(); }
  });
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    const body = text.value.trim();
    if (!body || send.disabled) return;
    key = key || newKey();
    send.disabled = true;
    hint.textContent = "Sending…";
    try {
      await api(`/v1/runs/${id}/messages`, {
        method: "POST",
        headers: { "Idempotency-Key": key },
        body: JSON.stringify({ text: body }),
      });
      key = null;
      text.value = "";
      grow();
      hint.textContent = "Sent. It shows in the log.";
      setTimeout(() => { hint.textContent = base; }, 3000);
    } catch (err) {
      hint.textContent = base;
      showError(err);
    }
    send.disabled = false;
    text.focus();
  });

  return {
    form,
    update(state, harness) {
      const accepts = ["open", "waiting_approval", "needs_attention", "parked"].includes(state);
      form.hidden = !accepts;
      const who = harness === "codex" ? "Codex" : harness === "claude-code" ? "Claude Code" : null;
      base = state === "parked" ? "It is waiting for your reply."
        : state === "needs_attention" ? "Read when the run continues."
        : who ? `${who} reads this when it finishes its current step.`
        : "Read at its next turn.";
      if (hint.textContent !== "Sending…" && !hint.textContent.startsWith("Sent")) hint.textContent = base;
      text.placeholder = state === "parked" ? "Reply" : "Message";
    },
  };
}

// crypto.randomUUID needs a secure context; the office may be opened over
// plain http on the tailnet, where getRandomValues still works.
function newKey() {
  const b = crypto.getRandomValues(new Uint8Array(16));
  return Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
}
