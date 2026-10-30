// One run: the header (task, state, where it works), the structured log,
// the box for writing to it, and a side panel with the approval it waits
// on and the diff of its branch.

import { api, app, el, esc, isOpen, kind, mark, MOD, narrow, onLeave, short, showError, span, stateLabel, toast, who } from "../core.js";
import { describe } from "../describe.js";
import { getRun, upsert, watch } from "../office.js";
import { streamEvents } from "../stream.js";
import { renderDiff } from "./diff.js";
import { createLog } from "./log.js";

export async function renderRun(id) {
  app.innerHTML = "";
  const run = el(`<div class="run">
    <div class="pane">
      <header class="run-head">
        <a class="back" href="#">‹ Runs</a>
        <div class="run-title"><h1></h1><span class="pill"></span></div>
        <div class="crumbs"></div>
        <div class="actions"></div>
      </header>
      <div class="log-wrap"><div class="log" tabindex="0" aria-label="Log"></div><button class="jump" type="button">Jump to latest ↓</button></div>
    </div>
    <aside class="aside" hidden aria-label="Approval and diff">
      <div class="aside-h"><h2>Approval &amp; diff</h2>
        <span class="seg" role="group" aria-label="Long lines"><button type="button" data-wrap="0" aria-pressed="true">Scroll</button><button type="button" data-wrap="1" aria-pressed="false">Wrap</button></span>
        <button class="icon-btn" type="button" data-close aria-label="Close">✕</button></div>
      <div class="aside-scroll"><div class="ask-slot"></div><div class="diff-slot"><div class="sk" style="width:60%"></div></div></div>
    </aside>
  </div>`);
  app.appendChild(run);
  const $ = (s) => run.querySelector(s);
  const logBox = $(".log");
  const jump = $(".jump");
  const aside = $(".aside");
  const composer = messageBox(id);
  $(".pane").appendChild(composer.form);

  let view = getRun(id) || null;
  let harness = null;   // "claude-code" | "codex" | null, from the spec
  let isolation = null; // from the first sandbox marker
  let raw = false;
  let wrap = false;
  let lastId = -1;
  const events = [];

  // ---- log ------------------------------------------------------------
  const decide = async (approvalId, decision) => {
    try {
      await api(`/v1/approvals/${approvalId}`, { method: "POST", body: JSON.stringify({ decision }) });
      toast(decision === "allow" ? "Allowed" : "Denied");
    } catch (err) { showError(err); }
  };
  let log = createLog(logBox, { onDecide: decide });
  const atBottom = () => logBox.scrollHeight - logBox.scrollTop - logBox.clientHeight < 60;
  let follow = true;
  logBox.addEventListener("scroll", () => { follow = atBottom(); if (follow) jump.classList.remove("show"); });
  jump.addEventListener("click", () => { logBox.scrollTo({ top: logBox.scrollHeight, behavior: "smooth" }); });
  const toBottom = () => { if (follow) logBox.scrollTop = logBox.scrollHeight; else jump.classList.add("show"); };

  const rawLine = (ev) => el(`<div><span class="seq">#${ev.seq}</span><span>${esc(describe(ev))}</span></div>`);
  const rebuild = () => {
    logBox.innerHTML = "";
    if (raw) {
      const box = el(`<div class="raw"></div>`);
      for (const ev of events) box.appendChild(rawLine(ev));
      logBox.appendChild(box);
    } else {
      log = createLog(logBox, { onDecide: decide });
      for (const ev of events) log.add(ev);
      if (view) log.setStatus(view.status);
    }
    logBox.scrollTop = logBox.scrollHeight;
  };

  const onEvent = (ev) => {
    events.push(ev);
    const p = ev.payload;
    if (p.type === "run_created") { harness = p.spec?.runtime?.kind === "harness" ? p.spec.runtime.harness : null; paint(); }
    if (p.type === "sandbox_started" && !isolation) { isolation = p.level; paint(); }
    if (raw) logBox.querySelector(".raw")?.appendChild(rawLine(ev)); else log.add(ev);
    toBottom();
    // The office stream keeps the view current; this covers a dropped one.
    clearTimeout(refreshTimer);
    refreshTimer = setTimeout(refresh, 400);
    if (p.type === "approval_requested" || p.type === "tool_result") diffStale = true;
  };
  let refreshTimer = null;
  const refresh = async () => {
    try { const v = await api(`/v1/runs/${id}`); upsert(v); } catch (e) { showError(e); }
  };

  // ---- header, actions, panel ----------------------------------------
  let asideOpen = false;
  let asideTouched = false;
  let diffStale = true;
  const setAside = (open) => {
    asideOpen = open;
    aside.hidden = !open;
    $("[data-aside-toggle]")?.setAttribute("aria-pressed", String(open));
    if (open) loadDiff();
  };
  aside.querySelector("[data-close]").addEventListener("click", () => { asideTouched = true; setAside(false); });
  for (const b of aside.querySelectorAll("[data-wrap]")) {
    b.addEventListener("click", () => {
      wrap = b.dataset.wrap === "1";
      for (const x of aside.querySelectorAll("[data-wrap]")) x.setAttribute("aria-pressed", String(x === b));
      diffStale = true; loadDiff();
    });
  }
  let diffText = null;
  const loadDiff = async () => {
    const slot = $(".diff-slot");
    if (!view?.repo) { slot.innerHTML = `<div class="aside-empty">This run has no repository, so there is no diff.</div>`; return; }
    if (!diffStale && diffText !== null) return;
    diffStale = false;
    try {
      diffText = await api(`/v1/runs/${id}/diff`);
      renderDiff(slot, diffText, { wrap });
    } catch (err) { slot.innerHTML = `<div class="aside-empty">${esc(err.message)}</div>`; }
  };

  const renderAsk = () => {
    const slot = $(".ask-slot");
    const st = view?.status;
    if (st?.state !== "waiting_approval") { slot.innerHTML = ""; return; }
    if (slot.dataset.id === st.approval_id) return;
    slot.dataset.id = st.approval_id;
    slot.innerHTML = "";
    const card = el(`<div class="card ask">
      <div class="eyebrow">Needs you</div>
      <h3>${esc(who(view.runtime))} wants to go ahead</h3>
      <div class="cmd">${esc(st.summary)}</div>
      <div class="row"><button class="btn" type="button" data-d="deny">Deny</button><button class="btn allow" type="button" data-d="allow">Allow</button></div>
      <div class="foot">Also on your phone. The first answer wins.</div></div>`);
    for (const b of card.querySelectorAll("[data-d]")) b.addEventListener("click", () => { for (const x of card.querySelectorAll("button")) x.disabled = true; decide(st.approval_id, b.dataset.d); });
    slot.appendChild(card);
  };

  const paint = () => {
    if (!view) return;
    const st = view.status.state;
    $(".run-title h1").textContent = view.task || "(no task)";
    const pill = $(".pill");
    pill.className = `pill ${kind(st)}`;
    pill.innerHTML = `${mark(st)}${esc(stateLabel(st))}`;
    const end = isOpen(st) ? Date.now() : view.updated_at;
    const crumbs = [
      view.repo ? esc(view.repo) : null,
      view.branch ? `<code>${esc(view.branch)}</code>` : null,
      esc(who(harness || view.runtime)),
      view.machine ? `on ${esc(view.machine)}` : null,
      isolation && isolation !== "worktree" ? esc(isolation) : null,
      view.created_at ? esc(span(Math.max(0, end - view.created_at))) : null,
      `<span title="${esc(id)}">${esc(short(id))}</span>`,
      view.origin ? `follows up <a href="#run=${esc(view.origin)}">${esc(short(view.origin))}</a>` : null,
    ].filter(Boolean);
    $(".crumbs").innerHTML = crumbs.join(`<span class="sep">·</span>`);
    renderActions(st);
    composer.update(st, harness);
    if (!raw) log.setStatus(view.status);
    renderAsk();
    // An approval opens the panel on a wide screen, unless you closed it.
    if (st === "waiting_approval" && !narrow() && matchMedia("(min-width: 1181px)").matches && !asideOpen && !asideTouched) setAside(true);
    if (asideOpen && diffStale) loadDiff();
  };

  const renderActions = (st) => {
    const actions = $(".actions");
    const key = `${st}|${raw}|${asideOpen}|${Boolean(view?.repo)}`;
    if (actions.dataset.key === key) return;
    actions.dataset.key = key;
    actions.innerHTML = "";
    if (isOpen(st)) {
      const cancel = el(`<button class="btn" type="button">Cancel run</button>`);
      cancel.addEventListener("click", async () => {
        if (cancel.dataset.armed !== "1") {
          cancel.dataset.armed = "1"; cancel.classList.add("armed"); cancel.textContent = "Really cancel?";
          setTimeout(() => { cancel.dataset.armed = ""; cancel.classList.remove("armed"); cancel.textContent = "Cancel run"; }, 4000);
          return;
        }
        cancel.disabled = true;
        try { await api(`/v1/runs/${id}/cancel`, { method: "POST", body: JSON.stringify({ reason: "from the desk" }) }); toast("Cancelling…"); }
        catch (err) { showError(err); }
      });
      actions.appendChild(cancel);
    }
    if (st === "open") {
      const resume = el(`<button class="btn quiet" type="button" title="Start a worker on it if none is working">Resume</button>`);
      resume.addEventListener("click", async () => {
        resume.disabled = true;
        try { await api(`/v1/runs/${id}/resume`, { method: "POST" }); toast("Resumed"); } catch (err) { showError(err); }
      });
      actions.appendChild(resume);
    }
    actions.appendChild(el(`<span class="grow"></span>`));
    const rawBtn = el(`<button class="btn quiet" type="button" aria-pressed="${raw}" title="Every event, one line each">Raw</button>`);
    rawBtn.addEventListener("click", () => { raw = !raw; rebuild(); renderActions(view.status.state); });
    actions.appendChild(rawBtn);
    const panel = el(`<button class="btn" type="button" data-aside-toggle aria-pressed="${asideOpen}">${view?.status.state === "waiting_approval" ? mark("waiting_approval") : ""}Approval &amp; diff <kbd>D</kbd></button>`);
    panel.addEventListener("click", () => { asideTouched = true; setAside(!asideOpen); renderActions(view.status.state); });
    actions.appendChild(panel);
  };

  // ---- wiring -----------------------------------------------------------
  if (!view) await refresh();
  view = getRun(id) || view;
  const unwatch = watch(() => { const v = getRun(id); if (v) { view = v; paint(); } });
  paint();

  const escClose = (e) => { if (e.key === "Escape" && asideOpen && !document.querySelector(".scrim")) { asideTouched = true; setAside(false); renderActions(view.status.state); } };
  document.addEventListener("keydown", escClose);
  const ticker = setInterval(() => { log.tick?.(); if (view && isOpen(view.status.state)) paint(); }, 30000);
  const controller = new AbortController();
  onLeave(() => { controller.abort(); clearTimeout(refreshTimer); clearInterval(ticker); unwatch(); document.removeEventListener("keydown", escClose); });
  streamEvents(`/v1/runs/${id}/events/stream`, controller.signal, (frame) => {
    if (frame.id !== undefined) lastId = frame.id;
    if (frame.data) { try { onEvent(JSON.parse(frame.data)); } catch (_) { /* skip a bad frame */ } }
  }, () => lastId);
}

// Writing to the run. A message goes in at the run's next turn: a native
// run reads it before its next model call, a harness when its current
// process finishes, a parked run wakes for it. Once a Claude Code or Codex
// run has finished, the box follows it up instead: a new run on the same
// branch and session, which the view then opens.
function messageBox(id) {
  const form = el(`<form class="composer" hidden>
    <label class="hint" for="message"></label>
    <textarea id="message" rows="1" placeholder="Message" enterkeyhint="send"></textarea>
    <div class="row"><span class="note">Your message goes into the run's log. The rules still apply to what the agent does next.</span>
      <button class="btn primary" type="submit"><span class="label">Send</span> <kbd>${MOD}↵</kbd></button></div>
  </form>`);
  const text = form.querySelector("textarea");
  const send = form.querySelector("button");
  const label = send.querySelector(".label");
  const hint = form.querySelector(".hint");
  let base = "";
  let followUp = false;
  // One key per message, kept until it is accepted, so a retry after a
  // dropped connection can't deliver it twice.
  let key = null;

  const grow = () => { text.style.height = "auto"; text.style.height = `${Math.min(text.scrollHeight, 200)}px`; };
  text.addEventListener("input", () => { if (!text.value.trim()) key = null; grow(); });
  text.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); form.requestSubmit(); }
    if (e.key === "Escape") text.blur();
  });
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    const body = text.value.trim();
    if (!body || send.disabled) return;
    key = key || newKey();
    send.disabled = true;
    hint.textContent = followUp ? "Starting the follow-up…" : "Sending…";
    try {
      const reply = await api(`/v1/runs/${id}/${followUp ? "follow-up" : "messages"}`, {
        method: "POST",
        headers: { "Idempotency-Key": key },
        body: JSON.stringify({ text: body }),
      });
      key = null;
      if (followUp) { upsert(reply); location.hash = `run=${reply.id}`; return; }
      text.value = "";
      grow();
      toast("Sent");
    } catch (err) {
      showError(err);
    }
    hint.textContent = base;
    send.disabled = false;
    text.focus();
  });

  return {
    form,
    update(state, harness) {
      const accepts = ["open", "waiting_approval", "needs_attention", "parked"].includes(state);
      const name = harness === "codex" ? "Codex" : harness === "claude-code" ? "Claude Code" : null;
      const mode = !accepts && name !== null;
      if (mode !== followUp) key = null;
      followUp = mode;
      form.hidden = !accepts && !followUp;
      label.textContent = followUp ? "Follow up" : state === "parked" ? "Reply" : name ? "Send at next turn" : "Send";
      base = followUp ? `Finished. A follow-up continues this ${name} session on the same branch, as a new run.`
        : state === "parked" ? "It is waiting for your reply."
        : state === "needs_attention" ? "It reads this when the run continues."
        : name ? `Tell ${name} something. It reads it when it finishes its current step.`
        : "Tell the agent something. It reads it at its next turn.";
      if (!send.disabled) hint.textContent = base;
      text.placeholder = followUp ? "What next?" : state === "parked" ? "Reply" : narrow() ? "Message" : "e.g. use the existing helper, don't add a dependency";
    },
  };
}

// crypto.randomUUID needs a secure context; the office may be opened over
// plain http on the tailnet, where getRandomValues still works.
function newKey() {
  const b = crypto.getRandomValues(new Uint8Array(16));
  return Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
}
