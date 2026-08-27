# Prompt Cult Proxy Protocol

This document is the binding contract for every provider proxy in this fork
(`codex-mistral-proxy` today; `codex-zen-proxy` and a future OpenCode Go proxy
next). A proxy that does not satisfy every **MUST** below is not a valid
Prompt Cult proxy and will be replaced with a compliant implementation.

## Status of existing proxies

| Proxy | ID | Status |
|---|---|---|
| `codex-mistral-proxy` | `proxy-mistral-ai` | Compliant (this spec was written for it) |
| `codex-zen-proxy` | `proxy-opencode-zen` | **Deprecated.** Predates this spec: no per-proxy config, no model filtering, no structured routing log. Scheduled to be rewritten as a compliant implementation; do not extend it further. |
| OpenCode Go proxy (planned) | `proxy-opencode-go` | Placeholder only; must be compliant from day one |

## 1. Identity

- Every proxy MUST have a unique, stable identifier from the shared
  `ProxyKind` enum. Defined values: `proxy-mistral-ai`, `proxy-opencode-zen`,
  `proxy-opencode-go`.
- Every log line a proxy emits MUST be prefixed with that identifier, so a
  user running several proxies can attribute output unambiguously.
- The enum is shared Rust code so identifiers cannot drift between proxies.

## 2. Per-proxy configuration

- A proxy MUST NOT read the main `config.toml` used by the TUI for
  proxy-specific settings. Each proxy owns exactly one config file:
  `$CODEX_CONFIG_DIR/<proxy-id>.jsonc`
  (e.g. `~/.codex-mistral/proxy-mistral-ai.jsonc`).
- The config directory is resolved from the `CODEX_CONFIG_DIR` environment
  variable, falling back to the default codex home.
- The file is JSONC (comments and trailing commas allowed). If the file is
  absent the proxy MUST start with compiled-in defaults. If the file is
  present but malformed the proxy MUST refuse to start with a clear error —
  silently ignoring a broken config is how "the wrong model ran" bugs happen.
- On startup, before serving, the proxy MUST log one line stating whether a
  config file was found (with its absolute path) or that defaults are in use.
- Recognised keys (all optional, defaults in brackets):
  - `upstream_base_url` — override the upstream API endpoint
    (Mistral: `https://api.mistral.ai/v1`).
  - `model_exclude_globs` — list of glob patterns; see §3
    (Mistral defaults: `["*-ocr-*", "*-mini-*", "magistral-*", "ministral-*",
    "voxtral-*", "glm-5-2"]`). An empty list disables filtering.
  - `log_level` — `"normal"` (default) or `"verbose"`; see §5.
  - `model_overrides` — map from upstream model ID to per-model metadata.
    Recognised fields: `base_instructions` (inline system instructions) or
    `base_instructions_file` (absolute path to a UTF-8 file, contents inlined;
    mutually exclusive with the inline form). Overrides are applied to the
    discovered `ModelInfo`, so codex uses them as the session's base
    instructions verbatim. Keys naming unknown/retired models are ignored.
    Use this to pin model identity when a provider canonicalises aliases —
    e.g. `zai-glm-5-2` requests being served by a backend that calls itself
    `mistral-code-agent-latest`.

## 3. Model discovery and filtering

- `GET /v1/models` MUST be routed regardless of any query string (codex always
  appends `?client_version=X`; exact-path matching is a bug).
- The response MUST be the codex `ModelsResponse` shape (`{"models":[…]}`),
  never a raw upstream relay. Codex decodes it strictly; a relayed upstream
  shape silently falls back to the bundled catalog.
- Discovery stays dynamic: the proxy queries the upstream catalog on every
  request so newly released provider models appear without a proxy release.
- Filtering is by **glob, applied to the model ID**:
  - `*` matches any run of characters (including none); there is no `**`.
  - A pattern with no `*` is an exact match (this is how the alias `glm-5-2`
    can be banned while `zai-glm-5-2` passes).
  - Matching is case-sensitive against the upstream model ID.
- A model matching ANY exclude glob is dropped. After translation the proxy
  MUST log `loaded N models, M after exclusions`.
- A proxy MUST serve each surviving model with codex base instructions. If
  the provider does not supply any, the proxy ships a default agentic prompt
  containing an identity line directing the model to answer identity questions
  with the selected model ID rather than an upstream alias name. See the
  Mistral proxy's `prompt.md` for the reference text.
- Regex was considered and rejected: globs cover prefix/suffix/contains/exact
  with no escaping hazards in a JSONC file.

## 4. Request forwarding

- `POST /v1/responses` MUST forward the model ID from the client request
  verbatim to the upstream call. A proxy MUST NOT substitute, "upgrade", or
  canonicalise the model slug. If the provider responds with a canonical name
  (e.g. Mistral aliases resolving to dated snapshots), the proxy MUST log both
  the requested and the upstream-reported model at verbose level, and MUST NOT
  rewrite the client-visible model identity.
- `POST /shutdown` (loopback only), `GET /health` MUST be supported; health
  returns the proxy ID and upstream base URL.

## 5. Logging

- Normal level: startup banner, config-found/defaults line, discovery totals,
  upstream non-200s, fatal errors.
- Verbose level: additionally one line per proxied `/v1/responses` request:
  proxy ID, a per-process monotonically increasing request counter, the
  requested model, and the upstream-reported model when known. This is the
  proof-of-model-switching channel.
- Logs MUST NEVER contain the API key, request bodies, or response bodies.
  The counter is per-process, not per-user; nothing is keyed by identity.

## 6. Security

- The API key comes from the environment (`MISTRAL_API_KEY`, loaded by the
  CLI's dotenvy) or stdin, into locked memory; it is injected into upstream
  requests and never logged or written to disk.
- The proxy binds loopback only and is never auto-spawned by the app.
