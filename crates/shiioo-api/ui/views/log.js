// The log as a conversation: your messages, the agent's words and its tool
// calls (one collapsible row each, with status and duration), approvals
// inline, and how the run ended. Plumbing events (leases, capacity, the
// sandbox marker) stay out; "Raw" in the run view shows every event.

import { ago, el, esc, exact, span, who } from "../core.js";
import { describe } from "../describe.js";

const FOLD = 12; // output lines shown before "show all"

/**
 * A log in `box`. `onDecide(approvalId, "allow" | "deny")` answers an
 * approval from its inline row.
 */
export function createLog(box, { onDecide }) {
  const inner = el(`<div class="log-inner"></div>`);
  box.appendChild(inner);
  let turn = null; // { kind, name, body }
  let agent = "Agent";
  let harness = false;
  let firstSession = true;
  let cost = 0;
  let started = 0;
  let status = null;
  const calls = new Map(); // call id -> { row, at }
  const waits = new Map(); // approval id -> row
  const unread = []; // { seq, state } of owner messages not yet read
  const working = el(`<div class="sys-row working-row"><span class="typing" aria-hidden="true"><i></i><i></i><i></i></span><span>Working</span></div>`);

  const header = (kind, name, ev, via) => {
    const letter = kind === "you" ? "Y" : kind === "sys" ? "·" : (name[0] || "A").toUpperCase();
    return `<div class="turn-h"><span class="who ${kind === "you" ? "" : kind}">${esc(letter)}</span><span class="name">${esc(name)}</span>
      <span class="when" data-ts="${ev.recorded_at}" title="${esc(exact(ev.recorded_at))}">${esc(ago(ev.recorded_at))}</span>${via ? `<span class="via">${esc(via)}</span>` : ""}</div>`;
  };
  const open = (kind, name, ev, via) => {
    if (turn && turn.kind === kind && turn.name === name && kind !== "you") return turn.body;
    const t = el(`<div class="turn">${header(kind, name, ev, via)}<div class="body"></div></div>`);
    inner.appendChild(t);
    turn = { kind, name, body: t.querySelector(".body") };
    return turn.body;
  };
  const sys = (ev, html, cls = "") => open("agent", agent, ev).appendChild(el(`<div class="sys-row ${cls}"><span class="dot"></span><span>${html}</span></div>`));
  const end = (ev, html, cls = "") => {
    turn = null;
    const row = el(`<div class="turn"><div class="end-row ${cls}">${html}</div></div>`);
    inner.appendChild(row);
  };

  // Calls whose process ended without answering them (interrupted,
  // crashed, or the run ended) stop spinning.
  const stopOpen = () => {
    for (const c of calls.values()) {
      if (c.done) continue;
      c.done = true;
      const x = c.row.querySelector(".exit");
      x.className = "exit"; x.textContent = "stopped";
    }
  };

  const markRead = (upTo) => {
    for (const m of unread.filter((m) => m.seq < upTo)) { m.state.textContent = "Read"; unread.splice(unread.indexOf(m), 1); }
  };

  function add(ev) {
    const p = ev.payload;
    switch (p.type) {
      case "run_created": {
        started = ev.recorded_at;
        const rt = p.spec?.runtime;
        harness = rt?.kind === "harness";
        agent = who(harness ? rt.harness : p.spec?.agent?.model);
        const body = open("you", "You", ev, p.spec?.origin ? "Follow-up" : null);
        const msg = el(`<div class="owner-msg">${esc(p.spec?.task || "")}</div>`);
        if (p.spec?.origin) msg.appendChild(el(`<span class="state">Continues <a href="#run=${esc(p.spec.origin.run)}">${esc(String(p.spec.origin.run).slice(0, 8))}</a> on the same branch and session</span>`));
        body.appendChild(msg);
        break;
      }
      case "user_message": {
        const mine = p.author === "owner";
        const body = open("you", mine ? "You" : p.author, ev);
        const msg = el(`<div class="owner-msg">${esc(p.content)}<span class="state">Not read yet</span></div>`);
        body.appendChild(msg);
        unread.push({ seq: ev.seq, state: msg.querySelector(".state") });
        break;
      }
      case "sandbox_started": if (harness) { markRead(ev.seq); stopOpen(); } break;
      case "harness_session_started": {
        if (firstSession) { firstSession = false; open("agent", agent, ev); }
        else sys(ev, "Picked the session up again");
        break;
      }
      case "model_response_recorded": {
        if (!harness) markRead(p.context_seq ?? ev.seq);
        const text = (p.response?.content || []).filter((b) => b.type === "text").map((b) => b.text).join("\n\n").trim();
        if (text) open("agent", agent, ev).appendChild(el(`<div class="prose">${md(text)}</div>`));
        break;
      }
      case "tool_intent": {
        const row = el(`<details class="call"><summary><span class="chev">▶</span><span class="tool">${esc(p.tool)}</span>
          <span class="arg">${esc(argOf(p.input))}</span><span class="exit run"><span class="spin"></span></span><span class="dur"></span></summary>
          <pre class="input">${esc(inputText(p.input))}</pre></details>`);
        open("agent", agent, ev).appendChild(row);
        calls.set(p.call_id, { row, at: ev.recorded_at });
        break;
      }
      case "tool_result": {
        const c = calls.get(p.call_id);
        if (!c) { sys(ev, esc(describe(ev))); break; }
        c.done = true;
        const exit = c.row.querySelector(".exit");
        exit.className = `exit ${p.is_error ? "bad" : "ok"}`;
        exit.textContent = p.is_error ? "error" : "ok";
        c.row.querySelector(".dur").textContent = span(Math.max(0, ev.recorded_at - c.at));
        c.row.appendChild(output(typeof p.output === "string" ? p.output : JSON.stringify(p.output, null, 2)));
        break;
      }
      case "tool_outcome_unknown": {
        const c = calls.get(p.call_id);
        if (c) { c.done = true; const x = c.row.querySelector(".exit"); x.className = "exit"; x.textContent = "unknown"; }
        break;
      }
      case "policy_decision":
        if (p.decision !== "allow") open("agent", agent, ev).appendChild(el(`<div class="rule-row">Rules: <b>${esc(p.decision)}</b> <code>${esc(p.tool)}</code> ${esc(p.reason || "")}</div>`));
        break;
      case "approval_requested": {
        const row = el(`<div class="wait-row"><span class="mk waiting"></span><span class="grow">Wants to <code>${esc(p.summary)}</code></span>
          <span class="actions"><button class="btn" type="button" data-d="deny">Deny</button><button class="btn allow" type="button" data-d="allow">Allow</button></span></div>`);
        for (const b of row.querySelectorAll("[data-d]")) b.addEventListener("click", () => { for (const x of row.querySelectorAll("button")) x.disabled = true; onDecide(p.approval_id, b.dataset.d); });
        open("agent", agent, ev).appendChild(row);
        waits.set(p.approval_id, row);
        sync();
        break;
      }
      case "approval_decided": {
        const row = waits.get(p.approval_id);
        const text = `${p.decision === "allow" ? "Allowed" : p.decision === "deny" ? "Denied" : esc(p.decision)} by ${esc(p.by === "owner" ? "you" : p.by)}`;
        const done = el(`<div class="resolved-row">${p.decision === "allow" ? "✓" : "✕"} ${text}${row ? `: <code>${esc(row.querySelector("code")?.textContent || "")}</code>` : ""}</div>`);
        if (row) { row.replaceWith(done); waits.delete(p.approval_id); } else open("agent", agent, ev).appendChild(done);
        break;
      }
      case "cancel_requested": sys(ev, `Cancel requested by ${esc(p.by === "owner" ? "you" : p.by)}`); break;
      case "interrupt_requested": sys(ev, `${p.by === "owner" ? "You" : esc(p.by)} stopped the current step so this is read now`); break;
      case "takeover_requested": sys(ev, "You asked for the keyboard; the agent stops its current step"); break;
      case "takeover_started": stopOpen(); sys(ev, `<b>You took over at the keyboard.</b> What you and the agent do there follows when you hand back.`, "keyboard"); break;
      case "takeover_ended": {
        markRead(ev.seq);
        stopOpen();
        const why = { handed_back: "You handed back", detached: "The takeover ended: nobody was attached for a while",
          exited: "The takeover ended: the agent's terminal closed", office_restarted: "The takeover ended: the office restarted",
          cancelled: "The takeover ended: the run was cancelled" }[p.reason] || `The takeover ended: ${esc(p.reason)}`;
        sys(ev, `<b>${esc(why)}.</b>`, "keyboard");
        break;
      }
      case "run_parked": if (p.waiting_for !== "takeover") sys(ev, "Waiting for your reply"); break;
      case "secret_redacted": sys(ev, `Kept secret <code>${esc(p.name)}</code> out of the log`); break;
      case "mail_sent": sys(ev, `Sent mail to ${esc((p.to || []).join(", "))}: ${esc(p.subject)}`); break;
      case "harness_result": cost += Number(p.cost_usd || 0); break;
      case "run_completed": stopOpen(); end(ev, `<b>Done</b> · ${esc(span(ev.recorded_at - started))}${cost >= 0.005 ? ` · $${cost.toFixed(2)}` : ""}`); break;
      case "run_failed": stopOpen(); end(ev, `<b>Failed</b> · ${esc(p.reason)}`, "failed"); break;
      case "run_cancelled": stopOpen(); end(ev, `<b>Cancelled</b>${p.reason ? ` · ${esc(p.reason)}` : ""}`); break;
      case "lease_acquired": case "capacity_observed": case "run_woken": break;
      default: sys(ev, esc(describe(ev)));
    }
    if (status === "open") inner.appendChild(working);
  }

  /** Enables only the approval the run is waiting on; shows "Working". */
  function sync() {
    for (const [id, row] of waits) {
      const live = status === "waiting_approval" && current?.approval_id === id;
      for (const b of row.querySelectorAll("button")) b.disabled = !live;
      row.querySelector(".actions").hidden = !live;
    }
    if (status === "open") inner.appendChild(working); else working.remove();
  }
  let current = null;
  function setStatus(st) {
    current = st;
    status = st.state;
    sync();
  }

  /** Refreshes the relative times. */
  function tick() {
    for (const w of inner.querySelectorAll(".when[data-ts]")) w.textContent = ago(Number(w.dataset.ts));
  }

  return { add, setStatus, tick };
}

function output(text) {
  const pre = el(`<pre class="out"></pre>`);
  const lines = text.replace(/\s+$/, "").split("\n");
  if (lines.length > FOLD) {
    const fold = el(`<button class="fold" type="button">Show all ${lines.length} lines</button>`);
    fold.addEventListener("click", () => { pre.textContent = lines.join("\n"); });
    pre.appendChild(fold);
    pre.appendChild(document.createTextNode(lines.slice(-FOLD).join("\n")));
  } else {
    pre.textContent = lines.join("\n") || "(no output)";
  }
  return pre;
}

/** The one argument that says what a call does. */
function argOf(input) {
  if (input == null) return "";
  if (typeof input !== "object") return String(input);
  if (Array.isArray(input.command)) return input.command.join(" ");
  for (const k of ["command", "cmd", "file_path", "path", "pattern", "url", "query", "description"]) {
    if (typeof input[k] === "string") return input[k];
  }
  const first = Object.values(input).find((v) => typeof v === "string");
  return first ?? JSON.stringify(input);
}

function inputText(input) {
  if (input && typeof input.command === "string" && Object.keys(input).length <= 2) return input.command;
  return JSON.stringify(input, null, 2);
}

/** A small, safe markdown subset: paragraphs, lists, headings, fenced and inline code, bold, links. */
export function md(src) {
  const out = [];
  // Fenced code first, then the text between the fences.
  const text = src.replace(/\r\n/g, "\n");
  const parts = [];
  const fence = /```[^\n]*\n([\s\S]*?)```/g;
  let last = 0;
  let m;
  while ((m = fence.exec(text))) {
    parts.push({ t: "text", v: text.slice(last, m.index) });
    parts.push({ t: "code", v: m[1] });
    last = fence.lastIndex;
  }
  parts.push({ t: "text", v: text.slice(last) });
  for (const part of parts) {
    if (part.t === "code") { out.push(`<pre><code>${esc(part.v.replace(/\n$/, ""))}</code></pre>`); continue; }
    for (const para of part.v.split(/\n{2,}/)) {
      const lines = para.split("\n").filter((l) => l.trim());
      if (!lines.length) continue;
      if (lines.every((l) => /^\s*[-*] /.test(l))) {
        out.push(`<ul>${lines.map((l) => `<li>${inline(l.replace(/^\s*[-*] /, ""))}</li>`).join("")}</ul>`);
      } else if (lines.every((l) => /^\s*\d+[.)] /.test(l))) {
        out.push(`<ol>${lines.map((l) => `<li>${inline(l.replace(/^\s*\d+[.)] /, ""))}</li>`).join("")}</ol>`);
      } else if (/^#{1,6} /.test(lines[0]) && lines.length === 1) {
        out.push(`<h4>${inline(lines[0].replace(/^#+ /, ""))}</h4>`);
      } else {
        out.push(`<p>${lines.map(inline).join("<br>")}</p>`);
      }
    }
  }
  return out.join("");
}

function inline(s) {
  return esc(s)
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g, '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>');
}
