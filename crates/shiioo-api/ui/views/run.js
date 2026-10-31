// One run: the header (task, state, where it works), the structured log,
// the box for writing to it, and a side panel with the approval it waits
// on and the diff of its branch. A view is an instance a desk pane mounts:
// it streams while it is shown and pauses when another tab covers it, so
// switching back is instant and resumes from the last event it has.

import { api, el, esc, isOpen, kind, mark, MOD, narrow, short, showError, span, stateLabel, toast, who } from "../core.js";
import { describe } from "../describe.js";
import { getRun, upsert, watch } from "../office.js";
import { liveStream } from "../stream.js";
import { renderDiff } from "./diff.js";
import { createLog } from "./log.js";

/** Room the side panel needs beside the log; narrower, it slides over. */
const INLINE_ASIDE = 1000;

export function createRunView(id) {
  const root = el(`<div class="run" data-run="${esc(id)}">
    <div class="run-main">
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
        <button class="icon-btn" type="button" data-close aria-label="Close" title="Close (Esc)">✕</button></div>
      <div class="aside-scroll"><div class="ask-slot"></div><div class="diff-slot"><div class="sk" style="width:60%"></div><div class="sk" style="width:85%"></div><div class="sk" style="width:40%"></div></div></div>
    </aside>
  </div>`);
  const $ = (s) => root.querySelector(s);
  const logBox = $(".log");
  const jump = $(".jump");
  const aside = $(".aside");
  const composer = messageBox(id);
  $(".run-main").appendChild(composer.form);

  let view = getRun(id) || null;
  let harness = null;   // "claude-code" | "codex" | null, from the spec
  let isolation = null; // from the first sandbox marker
  let raw = false;
  let wrap = false;
  let lastId = -1;
  let active = false;
  let saved = null; // the log's scroll while hidden: { top, follow }
  const events = [];

  // ---- log ------------------------------------------------------------
  const decide = async (approvalId, decision) => {
    try {
      await api(`/v1/approvals/${approvalId}`, { method: "POST", body: JSON.stringify({ decision }) });
      toast(decision === "allow" ? "Allowed" : "Denied");
      return true;
    } catch (err) { showError(err); return false; }
  };
  let log = createLog(logBox, { onDecide: decide });
  const atBottom = () => logBox.scrollHeight - logBox.scrollTop - logBox.clientHeight < 60;
  let follow = true;
  logBox.addEventListener("scroll", () => { if (!active) return; follow = atBottom(); if (follow) jump.classList.remove("show"); });
  jump.addEventListener("click", () => { logBox.scrollTo({ top: logBox.scrollHeight, behavior: "smooth" }); });
  const toBottom = () => { if (!active) return; if (follow) logBox.scrollTop = logBox.scrollHeight; else jump.classList.add("show"); };

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

  let refreshTimer = null;
  const refresh = async () => {
    try { const v = await api(`/v1/runs/${id}`); upsert(v); } catch (e) { showError(e); }
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

  // ---- header, actions, panel ----------------------------------------
  let asideOpen = false;
  let asideTouched = false;
  let diffStale = true;
  let diffText = null;
  const setAside = (open) => {
    asideOpen = open;
    aside.hidden = !open;
    root.classList.toggle("aside-open", open);
    if (open) loadDiff();
    if (view) renderActions(view.status.state);
  };
  aside.querySelector("[data-close]").addEventListener("click", () => { asideTouched = true; setAside(false); });
  for (const b of aside.querySelectorAll("[data-wrap]")) {
    b.addEventListener("click", () => {
      wrap = b.dataset.wrap === "1";
      for (const x of aside.querySelectorAll("[data-wrap]")) x.setAttribute("aria-pressed", String(x === b));
      diffStale = true; loadDiff();
    });
  }
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
    if (st?.state !== "waiting_approval") { slot.innerHTML = ""; delete slot.dataset.id; return; }
    if (slot.dataset.id === st.approval_id) return;
    slot.dataset.id = st.approval_id;
    slot.innerHTML = "";
    const card = el(`<div class="card ask" tabindex="-1">
      <div class="eyebrow">Needs you</div>
      <h3>${esc(who(view.runtime))} wants to go ahead</h3>
      <div class="cmd">${esc(st.summary)}</div>
      <div class="row"><button class="btn" type="button" data-d="deny">Deny</button><button class="btn allow" type="button" data-d="allow">Allow</button></div>
      <div class="foot">Also on your phone. The first answer wins.</div></div>`);
    for (const b of card.querySelectorAll("[data-d]")) b.addEventListener("click", () => answer(b.dataset.d));
    slot.appendChild(card);
  };
  // Answers the approval the run waits on: the card greys out at once and
  // comes back if the office refuses.
  const answer = async (decision) => {
    const st = view?.status;
    if (st?.state !== "waiting_approval") return false;
    const buttons = $(".ask-slot")?.querySelectorAll("button") || [];
    for (const x of buttons) x.disabled = true;
    const ok = await decide(st.approval_id, decision);
    if (!ok) for (const x of buttons) x.disabled = false;
    return ok;
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
    // An approval opens the panel when there is room for it beside the
    // log, unless you closed it.
    if (st === "waiting_approval" && active && !asideOpen && !asideTouched && root.clientWidth >= INLINE_ASIDE) setAside(true);
    if (asideOpen && diffStale) loadDiff();
  };

  const cancel = async () => {
    try { await api(`/v1/runs/${id}/cancel`, { method: "POST", body: JSON.stringify({ reason: "from the desk" }) }); toast("Cancelling…"); return true; }
    catch (err) { showError(err); return false; }
  };
  const resume = async () => {
    try { await api(`/v1/runs/${id}/resume`, { method: "POST" }); toast("Resumed"); } catch (err) { showError(err); }
  };
  const toggleRaw = () => { raw = !raw; rebuild(); if (view) renderActions(view.status.state); };

  const renderActions = (st) => {
    const actions = $(".actions");
    const key = `${st}|${raw}|${asideOpen}|${Boolean(view?.repo)}`;
    if (actions.dataset.key === key) return;
    actions.dataset.key = key;
    actions.innerHTML = "";
    if (isOpen(st)) {
      const btn = el(`<button class="btn" type="button">Cancel run</button>`);
      btn.addEventListener("click", async () => {
        if (btn.dataset.armed !== "1") {
          btn.dataset.armed = "1"; btn.classList.add("armed"); btn.textContent = "Really cancel?";
          setTimeout(() => { btn.dataset.armed = ""; btn.classList.remove("armed"); btn.textContent = "Cancel run"; }, 4000);
          return;
        }
        btn.disabled = true;
        if (!(await cancel())) btn.disabled = false;
      });
      actions.appendChild(btn);
    }
    if (st === "open") {
      const btn = el(`<button class="btn quiet" type="button" title="Start a worker on it if none is working">Resume</button>`);
      btn.addEventListener("click", async () => { btn.disabled = true; await resume(); btn.disabled = false; });
      actions.appendChild(btn);
    }
    actions.appendChild(el(`<span class="grow"></span>`));
    const rawBtn = el(`<button class="btn quiet" type="button" aria-pressed="${raw}" title="Every event, one line each">Raw</button>`);
    rawBtn.addEventListener("click", toggleRaw);
    actions.appendChild(rawBtn);
    const panel = el(`<button class="btn" type="button" data-aside-toggle aria-pressed="${asideOpen}">${st === "waiting_approval" ? mark(st) : ""}Approval &amp; diff <kbd>D</kbd></button>`);
    panel.addEventListener("click", () => { asideTouched = true; setAside(!asideOpen); });
    actions.appendChild(panel);
  };

  // ---- wiring -----------------------------------------------------------
  const unwatch = watch(() => { const v = getRun(id); if (v) { view = v; paint(); } });
  if (!view) refresh();
  paint();
  let ticker = null;
  const stream = liveStream(`/v1/runs/${id}/events/stream`, (frame) => {
    if (frame.id !== undefined) lastId = frame.id;
    if (frame.data) { try { onEvent(JSON.parse(frame.data)); } catch (_) { /* skip a bad frame */ } }
  }, () => lastId);
  stream.pause();

  return {
    id,
    root,
    get view() { return view; },
    /** Shown in a pane: stream, keep the clock moving, restore the scroll. */
    activate() {
      if (active) return;
      active = true;
      stream.resume();
      ticker = setInterval(() => { log.tick?.(); if (view && isOpen(view.status.state)) paint(); }, 30000);
      requestAnimationFrame(() => {
        if (!saved || saved.follow) logBox.scrollTop = logBox.scrollHeight;
        else logBox.scrollTop = saved.top;
        follow = saved ? saved.follow : true;
        paint();
      });
    },
    /** Covered by another tab: let go of the stream, remember the scroll. */
    deactivate() {
      if (!active) return;
      saved = { top: logBox.scrollTop, follow: atBottom() };
      active = false;
      stream.pause();
      clearInterval(ticker);
    },
    destroy() {
      this.deactivate();
      stream.stop();
      clearTimeout(refreshTimer);
      unwatch();
      root.remove();
    },
    /** Where the keyboard goes when the pane is focused. */
    focus() { logBox.focus({ preventScroll: true }); },
    focusReply() { return composer.focus(); },
    toggleAside() { asideTouched = true; setAside(!asideOpen); },
    /** Esc: closes the side panel; false when there was nothing to close. */
    escape() { if (!asideOpen) return false; asideTouched = true; setAside(false); return true; },
    /** The approval this run waits on, if any: `{ id, summary }`. */
    pending() { const st = view?.status; return st?.state === "waiting_approval" ? { id: st.approval_id, summary: st.summary } : null; },
    /** Opens the panel and puts the focus on Allow; Enter answers. */
    focusApproval() {
      if (!this.pending()) return false;
      if (!asideOpen) setAside(true);
      requestAnimationFrame(() => $(".ask-slot .btn.allow")?.focus());
      return true;
    },
    answer,
    cancel,
    resume,
    toggleRaw,
  };
}

// Writing to the run. A message goes in at the run's next turn: a native
// run reads it before its next model call, a harness when its current
// process finishes, a parked run wakes for it. Once a Claude Code or Codex
// run has finished, the box follows it up instead: a new run on the same
// branch and session, which the view then opens.
function messageBox(id) {
  const form = el(`<form class="composer" hidden>
    <label class="hint" for="message-${esc(id)}"></label>
    <textarea id="message-${esc(id)}" rows="1" placeholder="Message" enterkeyhint="send"></textarea>
    <div class="row"><span class="note">Your message goes into the run's log. The rules still apply to what the agent does next.</span>
      <button class="btn now" type="submit" data-now hidden title="Stop the current step and deliver this now">Now <kbd>${MOD}⇧↵</kbd></button>
      <button class="btn primary" type="submit"><span class="label">Send</span> <kbd>${MOD}↵</kbd></button></div>
  </form>`);
  const text = form.querySelector("textarea");
  const send = form.querySelector(".btn.primary");
  const nowBtn = form.querySelector("[data-now]");
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
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      form.requestSubmit(e.shiftKey && !nowBtn.hidden ? nowBtn : send);
    }
    if (e.key === "Escape") text.blur();
  });
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    const body = text.value.trim();
    if (!body || send.disabled) return;
    const now = e.submitter === nowBtn && !followUp;
    key = key || newKey();
    send.disabled = nowBtn.disabled = true;
    hint.textContent = followUp ? "Starting the follow-up…" : now ? "Stopping the current step…" : "Sending…";
    try {
      const reply = await api(`/v1/runs/${id}/${followUp ? "follow-up" : "messages"}`, {
        method: "POST",
        headers: { "Idempotency-Key": key },
        body: JSON.stringify(now ? { text: body, now } : { text: body }),
      });
      key = null;
      if (followUp) { upsert(reply); location.hash = `run=${reply.id}`; return; }
      text.value = "";
      grow();
      toast(now ? "Sent. Stopping the current step" : "Sent");
    } catch (err) {
      showError(err);
    }
    hint.textContent = base;
    send.disabled = nowBtn.disabled = false;
    text.focus();
  });

  return {
    form,
    /** Puts the cursor in the box; false when the run takes no messages. */
    focus() { if (form.hidden) return false; text.focus(); return true; },
    update(state, harness) {
      const accepts = ["open", "waiting_approval", "needs_attention", "parked"].includes(state);
      const name = harness === "codex" ? "Codex" : harness === "claude-code" ? "Claude Code" : null;
      const mode = !accepts && name !== null;
      if (mode !== followUp) key = null;
      followUp = mode;
      form.hidden = !accepts && !followUp;
      // Now only means something while a harness is in a step.
      nowBtn.hidden = !(name && state === "open");
      label.textContent = followUp ? "Follow up" : state === "parked" ? "Reply" : name ? "Send at next turn" : "Send";
      base = followUp ? `Finished. A follow-up continues this ${name} session on the same branch, as a new run.`
        : state === "parked" ? "It is waiting for your reply."
        : state === "needs_attention" ? "It reads this when the run continues."
        : name && state === "open" ? `Tell ${name} something. It reads it when it finishes its current step, or at once with Now.`
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
