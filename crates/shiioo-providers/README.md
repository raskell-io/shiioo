# shiioo-providers

Model providers for Shiioo's native agent loop beyond the built-in Anthropic one: a client for any server that speaks OpenAI's Chat Completions API (Ollama, LM Studio, `mlx_lm.server`, vLLM, OpenAI itself) and a router that picks a provider from a model-name prefix such as `ollama/qwen3:32b`. The recorded transcript format stays Anthropic-shaped (`text` and `tool_use` content blocks); this crate converts to and from the Chat Completions shapes at the edge.

## Where it sits

- Depends on: `shiioo-runtime` (the `ModelProvider` trait, `ModelRequest`, `ModelResponse`, `Transcript`, `ToolSpec`, `ProviderError`), `reqwest`, `serde_json`, `tokio` (`time`). Dev: `axum` for a fake server.
- Used by: the `shiioo` binary, which builds the office's router with `from_env` and hands it to the runtime.

## Public API

| Item | What it is |
|---|---|
| `OpenAiCompatProvider` | A non-streaming Chat Completions client implementing `ModelProvider`. Field `max_tokens: Option<u32>` is sent as `max_tokens` when set. |
| `OpenAiCompatProvider::new(base_url, api_key)` | `base_url` includes `/v1`; a trailing slash is tolerated. `api_key` becomes a bearer token when present. |
| `::ollama()`, `::lm_studio()`, `::openai(key)` | Presets for `OLLAMA_URL`, `LM_STUDIO_URL`, `OPENAI_URL`. |
| `.with_timeout(Duration)`, `.base_url()` | Request timeout for the HTTP client; the configured root. |
| `OLLAMA_URL`, `LM_STUDIO_URL`, `OPENAI_URL` | `http://127.0.0.1:11434/v1`, `http://127.0.0.1:1234/v1`, `https://api.openai.com/v1`. |
| `ProviderRouter` | `new(default)`, `route(prefix, provider)`, `prefixes()`, `select(model) -> (&provider, stripped_model)`. Implements `ModelProvider`. |
| `from_env(default) -> ProviderRouter` | The default plus `ollama/`, `lmstudio/`, and `openai/` when `OPENAI_API_KEY` is set. |

## Using it

```rust
use shiioo_providers::{OpenAiCompatProvider, ProviderRouter};
use std::sync::Arc;

let router = ProviderRouter::new(anthropic)              // Arc<dyn ModelProvider>
    .route("ollama/", Arc::new(OpenAiCompatProvider::ollama()))
    .route("ollama/big/", Arc::new(OpenAiCompatProvider::new("http://spark:11434/v1", None)));

let (_provider, model) = router.select("ollama/big/qwen3:235b");
assert_eq!(model, "qwen3:235b");                         // longest prefix wins, prefix stripped
let (_provider, model) = router.select("claude-sonnet-4-5");
assert_eq!(model, "claude-sonnet-4-5");                  // no prefix: default, unchanged
```

The same behaviour is pinned by the unit test `router_picks_the_longest_prefix_and_strips_it`.

## Environment

| Variable | Effect in `from_env` |
|---|---|
| `SHIIOO_OLLAMA_URL` | Base URL for `ollama/` models (default `OLLAMA_URL`). `OLLAMA_HOST` is not consulted. |
| `SHIIOO_LMSTUDIO_URL` | Base URL for `lmstudio/` models (default `LM_STUDIO_URL`). |
| `OPENAI_API_KEY` | When non-empty, registers `openai/` against `OPENAI_URL` with this key. |

## Testing

```sh
cargo test -p shiioo-providers
```

Unit tests cover the router, the request shape and response edge cases (missing ids, object arguments). `tests/openai_compat.rs` starts an `axum` fake on a random port and checks a tool-call turn round trips, a 429 is retried, a 400 is not, and the router strips the prefix before sending.

## Invariants

- Non-streaming: the whole response is recorded at once, like every provider in Shiioo today.
- A request is retried at most 4 times, only on connection errors and on HTTP 408, 409, 429 and 5xx. A `Retry-After` header is honoured up to 60 s.
- Tool-call ids from the server are kept; an empty or missing id becomes `call_<index>` so the loop always has one per call.
- A tool result's `is_error` has no equivalent in Chat Completions, so it is said in the text: the content is prefixed with `Error: `.

Full mapping tables: [`docs/openai-compat.md`](docs/openai-compat.md).

## Further reading

- [Runtimes and providers design](../../docs/design/runtimes-and-providers.md), [architecture overview](../../docs/architecture.md).
- [ADR-0009 Harness-first runtimes](../../docs/adr/0009-harness-first-runtimes.md): why the native loop and its providers are the second path, not the first.
