# codex-zen-proxy(1)

## NAME

`codex-zen-proxy` — translating API proxy for OpenCode Zen

## SYNOPSIS

```
printenv OPENCODE_API_KEY | codex-zen-proxy [--port PORT] [--upstream-base URL] [--server-info FILE] [--http-shutdown]
```

## DESCRIPTION

`codex-zen-proxy` is a sidecar process that sits between a harness and the
OpenCode Zen API.  It accepts the OpenAI Responses wire API (which the
harness speaks via `api_style = "openai-responses"`) and routes each request
to the correct Zen endpoint based on model family:

```
harness  ──POST /v1/responses──▶  codex-zen-proxy (127.0.0.1:9099)
                                        │
                          ┌─────────────┴──────────────┐
                    gpt-* │                             │ claude-*
                          ▼                             ▼
              /zen/v1/responses              /zen/v1/messages
              (passthrough)          (OAI Responses ↔ Anthropic
                                      Messages translation)
```

The API key is read from stdin at startup, stored in `mlock(2)`-protected
memory, and injected into every upstream request.  It is never written to
disk or passed through environment variables.

## OPTIONS

`--port PORT`
: TCP port to listen on (default: ephemeral; write `--server-info` to
  discover it).

`--upstream-base URL`
: Base URL of the Zen API (default: `https://opencode.ai/zen/v1`).
  GPT requests go to `{base}/responses`; Claude requests to `{base}/messages`.

`--server-info FILE`
: Write `{"port": N, "pid": N}` to FILE once the listener is ready.
  Useful for scripted startup where you need the ephemeral port.

`--http-shutdown`
: Enable a `GET /shutdown` endpoint that causes the process to exit cleanly.

## ENDPOINTS SERVED

`POST /v1/responses`
: The only request path the harness ever sends.  All other paths return 403.

`GET /health`
: Returns `{"status":"ok","proxy":"proxy-opencode-zen","upstream":"..."}`.

## SECURITY

- Key is read via raw `read(2)` to avoid `BufReader` retaining a copy.
- The stack buffer holding the raw bytes is zeroized immediately after use.
- The final `"Bearer <key>"` string is heap-allocated once, then `mlock(2)`'d
  to prevent it being swapped to disk.
- The key is validated to contain only `[A-Za-z0-9\-_]` before use.

## CONFIGURATION

The harness carries no key.  In `~/.vibe/config.toml` (or a project
`.vibe/config.toml`) point a provider at the loopback proxy and leave the
key env var empty:

```toml
[[providers]]
name = "zen-proxy"
api_base = "http://127.0.0.1:9099/v1"
api_key_env_var = ""          # the proxy holds the key; the harness must not
api_style = "openai-responses"
backend = "generic"

[[models]]
name = "gpt-5.4"              # or any claude-* model
provider = "zen-proxy"
alias = "gpt-5.4-secure"
```

**Do not set `api_key_env_var` here.**  The proxy holds the API key in
locked memory and injects it into every upstream request.  The harness
itself must have no knowledge of the key — that is the entire point of the
privilege-separation model.  Setting an env var would cause the harness to
demand the secret in its own environment, defeating the security design.

## USAGE

```bash
# start the proxy (stays in foreground; use a second terminal or background it)
printenv OPENCODE_API_KEY | ./codex-rs/target/debug/codex-zen-proxy --port 9099 &

# or with the installed binary
printenv OPENCODE_API_KEY | codex-zen-proxy --port 9099 &

# then run the harness as normal
```

## CRATE

`codex-rs/zen-proxy` — standalone binary `codex-zen-proxy`.
