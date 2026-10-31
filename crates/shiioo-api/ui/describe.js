// One line per event, mirroring the server's describe().

import { trunc } from "./core.js";

export function describe(ev) {
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
    case "user_message": return `${p.author === "owner" ? "you" : p.author}: ${trunc(p.content, 140)}`;
    case "policy_decision": return `rules: ${p.decision} ${p.tool} (${p.reason})`;
    case "approval_requested": return `waiting for approval: ${p.summary}`;
    case "approval_decided": return `${p.decision} by ${p.by}`;
    case "cancel_requested": return `cancel requested by ${p.by}${p.reason ? ": " + p.reason : ""}`;
    case "interrupt_requested": return `interrupt requested by ${p.by}`;
    case "takeover_requested": return `takeover requested by ${p.by}`;
    case "takeover_started": return `${p.by} took over at the keyboard`;
    case "takeover_ended": return `takeover ended (${p.reason})${p.session ? `, session ${p.session}` : ""}`;
    case "sandbox_started": {
      const parts = [p.detail?.container, p.detail?.sandbox, p.detail?.egress?.mode ? `egress ${p.detail.egress.mode}` : null].filter(Boolean);
      return `isolation: ${p.level}${parts.length ? ` (${parts.join(", ")})` : ""}`;
    }
    case "secret_redacted": return `redacted secret ${p.name} (${p.count} occurrence(s))`;
    case "mail_sent": return `mail sent to ${(p.to || []).join(", ")}: ${p.subject}`;
    case "run_parked": return `parked, waiting for ${p.waiting_for}`;
    case "run_woken": return `woken by ${p.reason}`;
    case "run_completed": return `completed: ${trunc(typeof p.output === "string" ? p.output : JSON.stringify(p.output), 140)}`;
    case "run_failed": return `failed: ${p.reason}`;
    case "run_cancelled": return `cancelled: ${p.reason}`;
    default: return p.type;
  }
}
