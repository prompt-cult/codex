# codex-opencode-proxy(1)

## NAME

`codex-opencode-proxy` — translating API proxy for OpenCode Zen and Go

## SYNOPSIS

```
export OPENCODE_API_KEY=...   # or pipe the key on stdin
codex-opencode-proxy [--port PORT] [--upstream-base URL] [--server-info FILE] [--http-shutdown]
```

## DESCRIPTION

`codex-opencode-proxy` is a sidecar process that sits between a harness and
the OpenCode API. One binary, one key (`OPENCODE_API_KEY`), two upstream
bases, selected with `--upstream-base`:

- **Zen** (paid): `https://opencode.ai/zen/v1`
- **Go** (open): `https://opencode.ai/zen/go/v1`

It accepts the OpenAI Responses wire API (which the harness speaks via
`api_style = "openai-responses"`) and routes each request to the correct
upstream route based on model family:

```
harness  ──POST /v1/responses──▶  codex-opencode-proxy (127.0.0.1:9099)
                                         │
              ┌──────────────────────────┼───────────────────────────┐
   gpt-* o1* │            claude-*      │      glm-* kimi-* deepseek-*
   o3* grok-*│            minimax-*     │      longcat-* mimo-* hy-*
   muse-*    │            qwen*         │
             ▼                          ▼                           ▼
      {base}/responses            {base}/messages            {base}/chat/completions
      (passthrough)         (OAI Responses ↔            (OAI Responses ↔ OpenAI-
                             Anthropic Messages           compatible Chat Completions
                             translation)                 translation)
```

Both endpoints share the same OpenAI-ish `/models` list shape, so
`GET /v1/models` discovery works against either base.

The Go endpoint requires a non-generic `User-Agent` and a stable
`x-opencode-session` header per conversation. The proxy is the client: it
sends `User-Agent: codex-opencode-proxy/<version>` and one UUIDv4 generated
at startup on EVERY upstream request (zen and go alike; harmless for zen).
Client-supplied values of these two headers are never forwarded — the proxy
overrides them.

## OPTIONS

`--port PORT`
: TCP port to listen on (default: ephemeral; write `--server-info` to
  discover it).

`--upstream-base URL`
: Base URL of the OpenCode API. Overrides `upstream_base_url` in
  `proxy-opencode-zen.jsonc` / `proxy-opencode-go.jsonc`; compiled-in
  default: `https://opencode.ai/zen/v1`. A `/go` path segment selects the
  Go kind (log prefix, health JSON and the Go config file).

`--server-info FILE`
: Write `{"port": N, "pid": N}` to FILE once the listener is ready.
  Useful for scripted startup where you need the ephemeral port.

`--http-shutdown`
: Enable a `GET /shutdown` endpoint that causes the process to exit cleanly.

## ENDPOINTS SERVED

`POST /v1/responses`
: The request path the harness sends. Model-family routing as above;
  unclassified models get a 501.

`GET /v1/models` (also `/models`, query string ignored)
: Fetches `{base}/models` and translates the raw list into the codex
  `ModelsResponse` shape, filtering to chat-capable models minus the
  configured exclusion globs.

`GET /health`
: Returns `{"status":"ok","proxy":"proxy-opencode-zen|proxy-opencode-go","upstream":"..."}`.

All other paths return 403.

## API KEY & SECRET SUPPLY

The proxy implements Prompt Cult Proxy Protocol v1 (see `docs/proxy-protocol.md`):

1. **`secret-push` mode:** When spawned by `codex-proxy-router` or an orchestrator
   with `--secret-channel secret-push --boot-token-file <FILE>`, the proxy starts
   keyless, consumes the boot-token file (zeroized and deleted at startup), and
   receives the key via an authenticated `POST /protocol/v1/secrets` using the
   ephemeral single-use boot token.
2. **`workload-identity` mode:** Not implemented by this proxy; the channel is
   reserved for commercial proxies that resolve their own credentials. The proxy
   rejects `--secret-channel workload-identity` at startup and never advertises it.
3. **`env-debug` / Standalone mode:** For local development and testing, the key
   is read from the `OPENCODE_API_KEY` environment variable (the proxy unsets the
   variable immediately upon reading and zeroes the buffer), falling back to stdin
   via a low-level `read(2)` when run interactively.

The key is stored in `mlock(2)`-protected memory and injected into upstream requests.
It is never written to disk or logged.

## SECURITY

- Protocol v1 secret supply: supports direct memory push (`secret-push`) and
  immediate env-unsetting (`env-debug`); `workload-identity` is deliberately not
  advertised (see `docs/proxy-protocol.md` section 4.3).
- In `env-debug` mode, `OPENCODE_API_KEY` is cleared from the process environment
  (`std::env::remove_var`) immediately upon reading, before validation.
- Stdin fallback reads via raw `read(2)` so no `BufReader` retains a copy.
- Buffers holding raw key bytes are zeroized immediately after use.
- The final `"Bearer <key>"` string is heap-allocated once, then locked via `mlock(2)`
  to prevent it from being swapped to disk.
- Key material may be any non-empty run of printable ASCII (0x21–0x7e);
  whitespace and control characters are rejected.

## CONFIGURATION

The proxy reads an optional settings file from the codex config directory
(`CODEX_CONFIG_DIR` > `CODEX_HOME` > `~/.codex`):

- `proxy-opencode-zen.jsonc` — when the resolved upstream base is a Zen path
- `proxy-opencode-go.jsonc` — when the resolved upstream base path contains `/go`

Keys (all optional): `upstream_base_url`, `model_exclude_globs`,
`log_level` (`"normal" | "verbose"`), `model_overrides` (per-model
`base_instructions` / `base_instructions_file`). There is no default
exclusion glob; both endpoints curate their own lists.

## HARNESS SETUP

The harness carries no key. In `~/.vibe/config.toml` (or a project
`.vibe/config.toml`) point a provider at the loopback proxy and leave the
key env var empty:

Go base (open models, e.g. `glm-5.3-flash`) — boot the proxy with
`--upstream-base https://opencode.ai/zen/go/v1`:

```toml
[[providers]]
name = "opencode-go"
api_base = "http://127.0.0.1:9099/v1"
api_key_env_var = ""          # the proxy holds the key; the harness must not
api_style = "openai-responses"
backend = "generic"

[[models]]
name = "glm-5.3-flash"
provider = "opencode-go"
```

Zen base (paid models, e.g. `gpt-5.4-mini`, `claude-haiku-4-5`) — boot the
proxy with the default `--upstream-base https://opencode.ai/zen/v1`:

```toml
[[providers]]
name = "opencode-zen"
api_base = "http://127.0.0.1:9099/v1"
api_key_env_var = ""
api_style = "openai-responses"
backend = "generic"

[[models]]
name = "gpt-5.4-mini"
provider = "opencode-zen"

[[models]]
name = "claude-haiku-4-5"
provider = "opencode-zen"
```

**Do not set `api_key_env_var` to a real env var.** The proxy holds the API
key in locked memory and injects it into every upstream request. The harness
itself must have no knowledge of the key — that is the entire point of the
privilege-separation model.

## USAGE

```bash
export OPENCODE_API_KEY=...

# go base
codex-opencode-proxy --port 9099 --upstream-base https://opencode.ai/zen/go/v1 &

# or zen base
codex-opencode-proxy --port 9099 &

# then run the harness as normal
```

## CRATE

`codex-rs/opencode-proxy` — standalone binary `codex-opencode-proxy`.
