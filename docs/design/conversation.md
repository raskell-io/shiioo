# Conversation: talking to Shiioo

Status: **requirements draft, for the owner's review** (phase 7 candidate, ahead of [fork and replay](fork-and-replay.md)). Nothing here is built. Decisions marked **D1**…**D9** are open; the rest follows from the picture.

Related: [telegram.md](telegram.md) (the phone today), [execution.md §1](execution.md#1-run-states-partly-built) (parked runs), [api.md](api.md) (messages to a waiting run), [governance.md](governance.md) (rules, approvals), [runtimes-and-providers.md](runtimes-and-providers.md) (the native loop, local models), [security.md](security.md).

## 1. The picture

The owner is out, walking, with earbuds in. They talk to Shiioo the way they would talk to a chief of staff on the phone: half-formed ideas, questions about what is going on, "what if we…", and at some point "do it". Shiioo listens, asks back when something is vague, knows the state of the office (runs, repositories, capacity, open plans, branches awaiting review), proposes, and when the owner says go, hands the work to the right employee on the right machine. Later the result arrives the way results already do.

Three things change compared with today:

| Today | With a conversation |
|---|---|
| A plain-text message starts a run at once | A message goes to the chief of staff, who may ask back, propose, or delegate |
| Commands (`/status`, `/plan`, `/run`) are the interface | Commands still work; most of the time the owner just talks |
| Text only | Voice notes in, text and (optionally) voice notes out; a Siri path with no app at all |

Principles that carry over unchanged: every decision is on the record; the rules and the owner's approvals gate every side effect exactly as now; the owner's subscriptions and hardware do the work; nothing listens on a public address.

## 2. Scope

**In:**

- A conversational agent ("the chief of staff", the persona is "Shiioo") with office tools, holding a thread with the owner across messages and days.
- Telegram as the surface: text and voice notes in; text and voice notes out.
- A `POST /v1/chat` endpoint for the same conversation, for a Siri Shortcut and the dashboard.
- Delegation from the conversation: starting runs (with role, repo, machine), asking for plans, approving plans, approving or denying pending requests, cancelling runs, listing and sending files, the report.
- Local speech-to-text and text-to-speech on the office machine.
- The kernel pieces the conversation needs: a run that parks waiting for input and wakes on a message.

**Out (for this phase):**

- A phone app, a web chat UI beyond the dashboard, or a phone call (real-time duplex audio). Telegram voice notes are turn-based by design.
- Reading the owner's mail or calendar into the conversation.
- The conversation agent acting on its own without a prompt (routines and the report already cover scheduled speaking-first; see R10 for proactive turns *within* a thread).
- Multi-user conversations. One owner.

## 3. Requirements

### The conversation

- **R1. One thread, remembered.** Messages from the owner (typed or spoken) go to one current conversation thread. The thread persists across office restarts and across days: the agent can refer back to what was said yesterday. The owner can start a fresh thread (`/new`) and the old one stays readable in the record.
- **R2. The agent knows the office.** Before answering, the agent has, or can fetch with tools: running and recent runs with their state, the repositories and their labels, machines and whether they are online, capacity left this week, open approvals, proposed plans, branches awaiting review, the routines. It never answers "I don't know what's running" about its own office.
- **R3. Ping-pong before delegation.** The agent may ask clarifying questions, offer two or three options, or propose a plan. It delegates when the owner tells it to, in words ("do it", "go", "yes, Codex"), or when the owner has asked for direct action. **D1:** the default confirmation policy. Proposed: *confirm first*, with a per-thread switch ("just do things without asking") and a per-message escape ("just do it").
- **R4. Delegation is the existing machinery.** Starting a run uses the same assignment (employee broker, roles, isolation policy, machine selector) as `/run`; plans use `/plan`'s planning run; approvals use the same `decide`. The conversation adds no second way of doing anything. Every delegation is a tool call recorded in the conversation's own log, and the created run names the conversation as its origin.
- **R5. Results come back into the thread.** When a run the conversation started ends, the thread learns of it (so the next answer can say "the fix you asked for this morning is up as PR #42"), and the owner is told as today. **D2:** whether the agent should also *comment* on results unprompted (a short "that landed; want me to merge?") or stay silent until spoken to. Proposed: a one-line comment for runs it started, silence for everything else.
- **R6. Commands keep working.** `/status`, `/run`, `/plan`, `/approve` and the rest behave as today, inside or outside a conversation. A command is not sent to the agent. The Allow / Deny buttons are unchanged.
- **R7. Graceful degradation.** If the conversation model is unavailable (Ollama down, API key missing), the bot says so and plain text falls back to today's behaviour (start a run) after telling the owner it did. Nothing silently disappears.

### Voice

- **R8. Voice notes in.** A Telegram voice note is downloaded, transcribed on the office machine, and treated as a typed message. The transcript is shown back to the owner (quoted, short) so a mishearing is visible before it does damage. Transcription runs locally by default; **D3:** which model and runtime. Proposed: whisper.cpp with `large-v3-turbo` (about 1.6 GB, a 20-second note in about a second on Apple silicon), installed by Homebrew, with the model downloaded on first use.
- **R9. Voice notes out.** When the owner spoke, the agent's answer is sent as a voice note and as text; when the owner typed, text only (configurable). Speech is synthesized locally. **D4:** the voice. Proposed to start with macOS `say` (zero setup), with Kokoro as the better-sounding option once the pipeline works. Answers meant to be heard are written for the ear: short, no tables or code, numbers rounded, run ids as "the first run" or by task, not as hex.
- **R10. Interruptions and corrections.** "Stop", "cancel that", "no, I meant the other repo" are understood in context; a cancel reaches the run through the existing cancellation. Proactive turns inside a thread are limited to R5's result comments; the agent does not start talking on its own otherwise.

### Beyond Telegram

- **R11. A chat endpoint.** `POST /v1/chat { "text": "…" }` (and `GET /v1/chat` for the thread) speaks to the same conversation with the office token. **D5:** build the Siri Shortcut path in this phase (dictate → post → speak the reply)? It is small once the endpoint exists and gives "Hey Siri, tell Shiioo…" with no app. Proposed: yes, as documentation plus a downloadable Shortcut.
- **R12. Dashboard.** The dashboard shows the thread (read-only) and has a text box to continue it. No voice in the dashboard.

### Trust and record

- **R13. Everything on the record.** The thread is a run with the usual hash-chained log: every owner message, every transcript, every agent reply, every tool call and its result. `shiioo log <thread>` works. Secrets are redacted as everywhere.
- **R14. Rules unchanged.** The agent's tools that have side effects (start a run, approve, cancel, send a file) are the office's own operations with the owner at the keyboard, so they are not themselves gated by `rules.toml`; the runs they start are gated as always. The agent cannot approve a request on the owner's behalf unless the owner said so in that thread ("approve it").
- **R15. Local by default.** Transcription, synthesis and, by default, the conversation model run on the office machine. Audio files are kept only until transcribed or sent. **D6:** the conversation brain. Options: (a) a local model through the native loop (`ollama/…`), private and free, quality depends on the model; (b) Claude through an API key, better judgment, costs money per message; (c) Claude Code in print mode as the brain through an MCP server exposing the office tools, on the Max subscription, slower (five to twenty seconds a turn) and more moving parts. Proposed: (a) as the default with a named model, (b) as a config switch; (c) as an experiment later.

### Non-functional

- **R16. Latency.** A spoken question gets a spoken answer in under 20 seconds end to end on the Mac Studio with a 32B-class local model (transcribe ≤ 2 s, think ≤ 12 s, speak ≤ 3 s, transport). Typed text: under 10 seconds. The bot acknowledges a voice note within 2 seconds ("listening…" or a typing indicator) so the owner knows it arrived.
- **R17. Cost.** Nothing in the loop spends API credits by default. Delegated runs spend the subscriptions as today.
- **R18. Restart safety.** A thread survives `shiioo up` restarting mid-exchange: the owner's message is in the log before the agent is called; a lost reply is regenerated on the next drive.
- **R19. Memory footprint.** The local model and the speech models share the Mac with the harness runs; the configuration says which model, and the docs say what it costs in memory (a 32B model at 4-bit is about 20 GB).

## 4. Architecture sketch

**The thread is a run.** A conversation is a native-loop run with the chief-of-staff agent, kept open: after each answer the worker appends `RunParked { waiting_for: input }` and releases the lease; an owner message is appended by the server as `UserMessage` (external, no lease needed) and makes the run runnable; the next worker appends `RunWoken`, rebuilds the transcript from the log (owner messages, agent messages, tool calls and results), and takes a turn. This is the parked-run design of [execution.md §1](execution.md#1-run-states-partly-built), built for the first time; `POST /v1/runs/{run}/messages` from [api.md](api.md) is its API. One `[chat]` thread is "current"; `/new` parks the old one for good and starts another.

**The agent.** An `AgentSpec` with the persona and office instructions, running on the native loop with a tool set that exists only for conversations:

| Tool | Does | Side effect |
|---|---|---|
| `office_status` | runs, machines, capacity, approvals, plans, branches | none |
| `start_run` | `/run` with repo, role, machine, task | starts a run; records `ChildRunStarted` in the thread |
| `plan` / `approve_plan` | `/plan`, `/plan approve` | starts a planning run; creates issues |
| `decide` | allow or deny a pending request by id | answers a hook |
| `cancel_run` | `/cancel` | cancels |
| `files` / `send_file` | `/files`, `/get` | sends a document |
| `report` | the morning report text | none |
| `remember` | a note the thread keeps (preferences, standing instructions) | none |

**Delegation ties back.** `start_run` from a thread puts `origin: { thread, seq }` in the new run's spec; when that run ends, the server appends a `UserMessage { author: "office", content: "run 1a2b finished: …" }` to the thread (R5), which wakes it for a one-line comment if D2 says so.

**Voice pipeline** (`shiioo-voice`, a small crate): `voice` updates → `getFile` → download `.oga` → `whisper-cli -m <model> -f <file> --no-timestamps` → transcript; replies → `say -o reply.aiff` (or Kokoro) → `ffmpeg -c:a libopus reply.ogg` → `sendVoice`. The Telegram crate gains `download_file`, `send_voice` and a `voice` kind in `UpdateKind`; the bot gains "listening" acknowledgements.

**Brain.** The native loop's `ModelProvider` as it exists: `ollama/<model>`, `lmstudio/…`, `openai/…`, or Anthropic by API key. The persona and tool descriptions are the prompt; tool-calling quality is the main selection criterion for the local model (**D7:** which one; candidates to try on the Mac Studio: Qwen3-32B, Qwen3-72B/235B-A22B, Llama 3.3 70B, gpt-oss-120b).

**Configuration:**

```toml
[chat]
mode = "assistant"            # assistant: messages go to the chief of staff; direct: today's behaviour
model = "ollama/qwen3:32b"    # the brain; or an Anthropic model with ANTHROPIC_API_KEY
confirm = "ask"               # ask | act: delegate without a confirming turn
voice_reply = "when_spoken"   # when_spoken | always | never
transcriber = "whisper-cpp"   # path and model under [chat.whisper]
tts = "say"                   # say | kokoro
```

## 5. What it feels like

```
🎙 (voice) "the flaky network test in sango keeps failing, I think the retry loop is wrong"
   ↳ heard: "the flaky network test in sango keeps failing, I think the retry loop is wrong"
🔈 "That's tests/net.rs. Codex is free this week and knows the repo. Fix it on a
    branch and open a PR, or plan it first?"
🎙 "fix it"
🔈 "Handed to Codex on sango, run one-a-two-b. I'll tell you when the PR is up."
   … twenty minutes …
✅ 1a2b finished: fix the flaky test … Opened PR #42.
🔈 "The network test fix is up as PR 42. Want me to ask the reviewer role to look at it?"
```

## 6. Phasing

1. **7a. The thread, in text.** Parked runs and `UserMessage` wake-ups in the kernel; the chief-of-staff agent and its tools; `[chat] mode = assistant`; `/new`; delegation that ties back; `POST /v1/chat`. Testable with the scripted provider and a fake Telegram.
2. **7b. Voice in.** Telegram voice download, whisper.cpp, the "heard:" echo, the acknowledgement.
3. **7c. Voice out.** `say`, Opus conversion, `sendVoice`, ear-friendly phrasing in the persona; Kokoro as an option.
4. **7d. Siri Shortcut** and the dashboard thread view.
5. **7e. Live tuning** on the Mac Studio: model choice, latency, prompt.

## 7. Acceptance

- A typed message in assistant mode gets an answer that reflects the office's real state (a run named in the answer exists), and no run is started without the owner saying so when `confirm = ask`.
- "Do it" after a proposal starts exactly one run with the proposed repo, role and machine; the thread's log shows the tool call, the new run's spec names the thread.
- The office restarts between the owner's message and the answer; the answer still arrives, once.
- A voice note is transcribed, echoed, answered in text and voice within R16's budget on the Mac Studio; a typed message gets text only under `when_spoken`.
- `/status` during a conversation answers as today; the thread is unaffected.
- With Ollama stopped, a message gets "the chief of staff is unavailable (…); starting a run directly" and the run starts.
- `shiioo log <thread>` verifies and shows every message, transcript and tool call.

## 8. Decisions needed

| | Question | Proposed |
|---|---|---|
| D1 | Delegate only after a confirming turn, or act at once? | Confirm first; per-thread and per-message overrides |
| D2 | Does the agent comment when a run it started finishes? | One line, for its own runs only |
| D3 | Transcription model | whisper.cpp `large-v3-turbo`, local |
| D4 | Voice | macOS `say` first, Kokoro as the upgrade |
| D5 | Siri Shortcut in this phase | Yes, after the endpoint exists |
| D6 | The brain | Local model via the native loop by default; Anthropic by API key as a switch |
| D7 | Which local model | To be measured on the Mac Studio; Qwen3-32B as the first candidate |
| D8 | Assistant mode on by default for new offices? | On, since it is the point; `direct` keeps today's behaviour |
| D9 | Thread lifetime | One open thread until `/new`; the report does not reset it |
