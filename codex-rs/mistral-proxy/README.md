# codex-mistral-proxy

A standalone translating proxy that lets codex talk to the Mistral AI API. It
speaks the OpenAI Responses API on the loopback side (what codex expects) and
the Mistral Chat Completions API upstream. It mirrors the structure and security
model of `codex-zen-proxy`: the API key is read from stdin into
`mlock(2)`-protected memory and injected into upstream requests; the proxy is
never auto-spawned by the app — you boot it yourself.

Mistral-only by design: there is no model-family routing, so any model Mistral
exposes now or in the future works automatically.

## Running

```shell
grep '^MISTRAL_API_KEY=' /path/to/.env | cut -d= -f2 \
  | codex mistral-proxy --port 8901
# or, if the key is already exported in your shell:
printenv MISTRAL_API_KEY | codex mistral-proxy --port 8901
```

## HTTP contract

The proxy exposes exactly the surface codex needs:

- `POST /v1/responses` — OpenAI Responses request, translated to
  `{upstream}/chat/completions` and streamed back as Responses SSE.
- `GET /v1/models` — model discovery. **Codex always appends a query string**
  (`?client_version=X`), so the route match must ignore the query. The response
  is the codex `ModelsResponse` shape (`{"models":[…]}`), translated from
  Mistral's raw `{"object":"list","data":[…]}`. Only chat-capable models
  (`capabilities.completion_chat == true`) are returned.
- `GET /health` — liveness.
- `GET /shutdown` — only when `--http-shutdown` is set.

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
is reachable, the model picker lists Mistral's live models; when it is not, the
app falls back silently to the bundled catalog.
