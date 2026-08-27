# codex-mistral-proxy

Proxy ID: `proxy-mistral-ai`. Implements the
[Prompt Cult Proxy Protocol](../docs/proxy-protocol.md) — read that first; it
is the binding contract for this and every future proxy.

A standalone translating proxy that lets codex talk to the Mistral AI API. It
speaks the OpenAI Responses API on the loopback side (what codex expects) and
the Mistral Chat Completions API upstream. The API key is read from the
`MISTRAL_API_KEY` environment variable (falling back to stdin) into
`mlock(2)`-protected memory and injected into upstream requests; the proxy is
never auto-spawned by the app — you boot it yourself.

Mistral-only by design: there is no model-family routing, so any model Mistral
exposes now or in the future works automatically.

## Running

```shell
# key from the environment (dotenvy in the CLI already loads .env):
codex mistral-proxy --port 8901
# or piped explicitly:
grep '^MISTRAL_API_KEY=' /path/to/.env | cut -d= -f2 \
  | codex mistral-proxy --port 8901
```

## HTTP contract

The proxy exposes exactly the surface codex needs:

- `POST /v1/responses` — OpenAI Responses request, translated to
  `{upstream}/chat/completions` and streamed back as Responses SSE.
- `GET /v1/models` — model discovery. **Codex always appends a query string**
  (`?client_version=X`), so the route match must ignore the query. The response
  is the codex `ModelsResponse` shape (`{"models":[…]}`), translated from
  Mistral's raw `{"object":"list","data":[…]}`. Only chat-capable models
  (`capabilities.completion_chat == true`) are returned, minus any model whose
  ID matches an exclude glob (see below).
- `GET /health` — liveness.
- `GET /shutdown` — only when `--http-shutdown` is set.

## Proxy config (`proxy-mistral-ai.jsonc`)

The proxy reads `$CODEX_CONFIG_DIR/proxy-mistral-ai.jsonc` (default
`~/.codex/proxy-mistral-ai.jsonc`). The file is optional; without it the
defaults below apply. A malformed file is a startup error, never silently
ignored. Startup always logs whether the file was found.

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
  "log_level": "normal"
}
```

Discovery logs `loaded N models, M after exclusions` on every `/v1/models`
fetch so filtering is auditable.

## Model discovery contract

The main app discovers the model list from the proxy at startup when the active
provider points at a loopback proxy. Two rules keep this working:

1. The provider `base_url` must use a loopback host (`127.0.0.1` or `localhost`)
   — that is what `ModelProviderInfo::is_local_proxy()` matches, and it is what
   gates the discovery fetch.
2. The isolated config **must not** set a static `model_catalog`. A static
   catalog activates `CatalogMode::Custom`, which short-circuits the remote
   refresh, so the proxy is never queried.

## Isolated config

Point a frozen config directory at the proxy without disturbing the default
`~/.codex` install:

```shell
CODEX_CONFIG_DIR=$HOME/.codex-mistral codex
```

`~/.codex-mistral/config.toml` defines a provider with
`base_url = "http://127.0.0.1:8901/v1"` and no `model_catalog`. When the proxy
is reachable, the model picker lists Mistral's live models. When the proxy is
unreachable, the app surfaces a fetch error and shows no remote models — it
never silently substitutes the bundled GPT catalog for a local-proxy provider.
