# The OpenAI-compatible provider

How `OpenAiCompatProvider` turns a recorded transcript into a Chat Completions request and the reply back into the recorded format, what it retries, and how the router in front of it picks a provider.

## Endpoint

`POST {base_url}/chat/completions` with a JSON body, `Authorization: Bearer <key>` when a key was given. `base_url` is the API root including `/v1`.

| Preset | URL | Notes |
|---|---|---|
| `ollama()` | `http://127.0.0.1:11434/v1` | Ollama's OpenAI-compatible endpoint. |
| `lm_studio()` | `http://127.0.0.1:1234/v1` | LM Studio's local server. |
| `openai(key)` | `https://api.openai.com/v1` | Billed to the key. |
| `new(url, key)` | any | `mlx_lm.server`, vLLM, a remote Ollama, a DGX Spark on the Tailnet. |

## Request mapping

The transcript (`shiioo_runtime::Transcript`: `system`, `task`, `turns` of assistant blocks plus tool results) becomes `messages`:

| Transcript | Chat Completions |
|---|---|
| `system` (if non-empty) | `{ "role": "system", "content": system }` |
| `task` | `{ "role": "user", "content": task }` |
| Assistant `text` blocks of one turn | `content`: the texts joined with `\n`, or `null` when there is none |
| Assistant `tool_use` blocks | `tool_calls[]`: `{ "id", "type": "function", "function": { "name", "arguments": <input as a JSON string> } }` |
| Each tool result of the turn | `{ "role": "tool", "tool_call_id", "content" }`; `content` is prefixed with `Error: ` when `is_error` |
| `ModelRequest.tools` (if any) | `tools[]`: `{ "type": "function", "function": { "name", "description", "parameters": input_schema } }` and `"tool_choice": "auto"` |
| `max_tokens` on the provider (if set) | `max_tokens` |

`model` is the name after the router stripped its prefix.

## Response mapping

From `choices[0].message`:

| Chat Completions | Recorded block |
|---|---|
| `content` as a string, or an array of `{ "type": "text", "text" }` parts, non-empty | one `{ "type": "text", "text" }` block |
| `content` null or empty | no text block |
| each `tool_calls[i]` | `{ "type": "tool_use", "id", "name", "input" }` |

For `tool_use` blocks:

- `id` is the server's, or `call_<i>` when it is missing or empty (some local servers omit ids).
- `input` is the parsed `function.arguments` string; an unparsable string becomes `{ "_raw": "<string>" }`; `null` becomes `{}`; an object is used as is.
- `name` falls back to an empty string when absent.

| `finish_reason` | `StopReason` |
|---|---|
| `stop`, or absent | `EndTurn` |
| `tool_calls`, `function_call` | `ToolUse` |
| `length` | `MaxTokens` |
| `content_filter` | `Refusal` |
| anything else | `Other(reason)` |

`model` is taken from the response, falling back to the requested name when empty. `usage` maps `prompt_tokens` and `completion_tokens` to `input_tokens` and `output_tokens`, 0 when missing.

A response with no `choices` is a `ProviderError::Decode`; a body that is not JSON is a `Decode` with the first 300 characters of the body.

## Errors and retries

| Condition | Error | Retried |
|---|---|---|
| Connection failure, timeout | `ProviderError::Http` | yes |
| HTTP 408, 409, 429, 5xx | `ProviderError::Status { status, body }` | yes |
| Other non-2xx (400, 401, 404, …) | `ProviderError::Status` | no |
| Unparsable or empty response | `ProviderError::Decode` | no |

- At most `MAX_ATTEMPTS = 4` attempts. Between attempts the provider sleeps `500 ms × 2^attempt`: 1 s, 2 s, 4 s.
- When a retryable status carries `Retry-After: <seconds>`, the provider first sleeps that long (capped at 60 s), then also applies the exponential delay. A 429 with `Retry-After: 30` therefore waits 31 s before the second attempt.
- The HTTP timeout is per request (`with_timeout`); the reqwest default applies otherwise.

The integration tests `a_rate_limit_is_retried` and `a_bad_request_is_not_retried` cover the two sides.

## The router

`ProviderRouter` holds a default provider and a list of `(prefix, provider)` routes.

- `route(prefix, provider)` appends a `/` to the prefix when missing.
- `select(model)` picks the route with the longest prefix that `model` starts with and returns the provider together with the model name minus that prefix. No match: the default and the name unchanged.
- `complete` is `select` followed by the chosen provider's `complete` with the stripped name. `prefixes()` lists the routes for `/capacity`-style displays.

`from_env(default)` is what the office uses:

| Prefix | Provider | Configured by |
|---|---|---|
| (none) | `default`, normally the Anthropic provider | the server |
| `ollama/` | `OpenAiCompatProvider::new(SHIIOO_OLLAMA_URL or OLLAMA_URL, None)` | always |
| `lmstudio/` | `OpenAiCompatProvider::new(SHIIOO_LMSTUDIO_URL or LM_STUDIO_URL, None)` | always |
| `openai/` | `OpenAiCompatProvider::openai(key)` | only when `OPENAI_API_KEY` is non-empty |

Empty environment values count as unset.

## Not covered today

- Streaming responses; the loop records whole responses.
- Images or other non-text content in either direction.
- `tool_choice` other than `auto`, `temperature`, `seed` and other sampling knobs.
- Provider-specific extensions such as Ollama's `keep_alive` or `OLLAMA_HOST`.
- Capacity accounting for local servers; the office's capacity view is about subscription windows *(planned for providers with quotas)*.

Related: [runtimes and providers design](../../../docs/design/runtimes-and-providers.md).
