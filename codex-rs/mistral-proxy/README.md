# codex-mistral-proxy

Proxy ID: `proxy-mistral-ai`. Implements the Prompt Cult Proxy Protocol —
the shared contract lives in the `codex-rs/proxy-protocol` crate
(`proxy-protocol/src/lib.rs`); read that first, it is binding for this and
every future proxy.

A standalone translating proxy that lets a keyless harness talk to the
Mistral AI API. It speaks the OpenAI Responses API on the loopback side
(what a harness with `api_style = "openai-responses"` sends) and the
Mistral Chat Completions API upstream. The proxy implements Prompt Cult
Proxy Protocol v1 (see `docs/proxy-protocol.md`) with two secret supply
channels: `secret-push` (keyless boot; the key arrives via authenticated
`POST /protocol/v1/secrets` with an ephemeral single-use boot token delivered
as a 0600 file, never argv) and `env-debug`/standalone (the key read from the
`MISTRAL_API_KEY` environment variable, which is immediately unset upon
reading, or stdin when run interactively). `workload-identity` is not
implemented and never advertised — the channel is reserved for commercial
proxies that resolve their own credentials. The key lands in `mlock(2)`-
protected memory and is injected into upstream requests; it is never written
to disk or logged.
The proxy is never auto-spawned by the harness — you boot it yourself, e.g.
as a Docker sidecar. It can also be booted for you by `codex-proxy-router`
(see `README.proxy-router.md` at the repo root), which negotiates the secret
supply channel at boot per the protocol; the dispatcher never sees the
secret outside `env-debug` mode.

Mistral-only by design: there is no model-family routing, so any model Mistral
exposes now or in the future works automatically.

## Running

```shell
# key from the environment:
MISTRAL_API_KEY=… codex-mistral-proxy --port 8901
# or piped explicitly (read once at startup via read(2), never re-read):
printenv MISTRAL_API_KEY | codex-mistral-proxy --port 8901
```

## HTTP contract

The proxy exposes exactly the surface the harness needs:

- `POST /v1/responses` — OpenAI Responses request, translated to
  `{upstream}/chat/completions` and streamed back as Responses SSE.
- `GET /v1/models` — model discovery. **Clients may append a query string**
  (`?client_version=X`), so the route match ignores the query. The response
  is the harness `ModelsResponse` shape (`{"models":[…]}`), translated from
  Mistral's raw `{"object":"list","data":[…]}`. Only chat-capable models
  (`capabilities.completion_chat == true`) are returned, minus any model whose
  ID matches an exclude glob (see below). Models hidden from upstream
  discovery (e.g. `devstral-small-latest`) still serve when requested.
- `GET /health` — liveness.
- `GET /shutdown` — only when `--http-shutdown` is set.

## Proxy config (`proxy-mistral-ai.jsonc`)

The proxy reads `proxy-mistral-ai.jsonc` from the config directory resolved
as `CODEX_CONFIG_DIR`, then `CODEX_HOME`, then `$HOME/.codex`. The file is
optional; without it the defaults below apply. A malformed file is a startup
error, never silently ignored. Startup always logs whether the file was
found.

```jsonc
{
  // Upstream endpoint override:
  // "upstream_base_url": "https://api.mistral.ai/v1",

  // Glob patterns matched against the upstream model ID; matches are dropped
  // from discovery. "*" matches any run of characters; a pattern with no "*"
  // is an exact match. These are the defaults — set your own list to override
  // (an empty list disables filtering):
  "model_exclude_globs": [
    "*-ocr-*",      // OCR models are not chat models for our purposes
    "*-mini-*",     // voxtral-mini etc.
    "magistral-*",
    "ministral-*",  // 3B/8B/14B local-class models
    "voxtral-*",    // audio
    "glm-5-2"       // exact-match ban of the alias; zai-glm-5-2 still passes
  ],

  // "normal" (default) or "verbose" — verbose logs one line per proxied
  // /v1/responses request: request number, requested model, upstream model.
  "log_level": "normal",

  // Per-model metadata overrides, keyed by upstream model ID. Unknown keys
  // are ignored. base_instructions (inline) and base_instructions_file
  // (absolute path, contents inlined) are mutually exclusive. Use this to
  // pin identity for models whose backend answers with an alias name:
  // "model_overrides": {
  //   "zai-glm-5-2": { "base_instructions_file": "/path/to/glm-prompt.md" }
  // }
}
```

Without a per-model override, discovered models ship with the proxy's
default prompt (`mistral-proxy/prompt.md`), which directs the model to
answer "what model are you?" with the selected model ID.

Discovery logs `loaded N models, M after exclusions` on every `/v1/models`
fetch so filtering is auditable.

## Harness configuration

The harness carries no key. In `~/.vibe/config.toml` (or a project
`.vibe/config.toml`) point a provider at the loopback proxy and leave the
key env var empty:

```toml
[[providers]]
name = "secure-proxy"
api_base = "http://127.0.0.1:8901/v1"
api_key_env_var = ""          # the proxy holds the key; the harness must not
api_style = "openai-responses"
backend = "generic"

[[models]]
name = "zai-glm-5-2"
provider = "secure-proxy"
alias = "zai-glm-5-2-secure"
thinking = "off"
```

The compaction model must share the active model's provider, so give it a
sibling entry:

```toml
compaction_model = { name = "mistral-small-latest", provider = "secure-proxy", alias = "compact-secure", thinking = "off" }
```

When the proxy is unreachable the harness surfaces a request error — it
never silently falls back to another provider.
