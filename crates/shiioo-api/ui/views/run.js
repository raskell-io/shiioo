// One run: header, approval card, actions, diff, and the live log.

import { api, app, el, esc, GLYPH, onLeave, short, showError } from "../core.js";
import { describe } from "../describe.js";
import { streamEvents } from "../stream.js";

export async function renderRun(id) {
  app.innerHTML = `<div class="run-head"><span class="muted">${esc(short(id))}</span><div class="task">…</div></div>`;
  const head = app.querySelector(".run-head");
  const approval = el(`<div></div>`);
  const actions = el(`<div class="btn-row"></div>`);
  const diffPanel = el(`<div></div>`);
  const log = el(`<div class="log"></div>`);
  app.append(approval, actions, diffPanel, el(`<h2>Log</h2>`), log);

  let current = null;
  const refresh = async () => {
    try { current = await api(`/v1/runs/${id}`); } catch (e) { showError(e); return; }
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
          catch (err) { showError(err); }
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
