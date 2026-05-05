# OpenCode Zen — Codex Setup

This fork of codex-rs works with [OpenCode Zen](https://opencode.ai/zen) — a single API
that gives you GPT, Claude, Gemini and more.  We recommend them because they build great
open-source tooling and deserve the support.

## How it works

```
codex-rs  ──▶  zen_proxy (port 9099)  ──▶  opencode.ai/zen
                  │
                  ├─ GPT models   → /zen/v1/responses  (passthrough)
                  └─ Claude/etc   → /zen/v1/messages   (OAI↔Anthropic translation)
```

`codex-rs` only speaks the OpenAI Responses wire API.  The proxy translates everything
else so any Zen model just works.

## Prerequisites

| Tool | Why |
|------|-----|
| `uv` | runs both scripts with inline deps — no venv setup needed |
| `cargo` | to build codex-rs (binary already in `target/debug/codex`) |

Install `uv` if you don't have it: https://docs.astral.sh/uv/getting-started/installation/

## API key

Put your Zen key in `.env` (already there):

```
OPENCODE_API_KEY=sk-...
```

Keep it `chmod 600` and never commit it.  Get a key at https://opencode.ai/zen

## Quick start

```bash
# start the proxy (supervisord keeps it alive automatically)
./zenctl.py start

# check it's healthy
./zenctl.py status

# fire a one-shot joke at claude-haiku-4-5
./zenctl.py test

# run codex non-interactively
source .env && OPENCODE_API_KEY=$OPENCODE_API_KEY \
  ./codex-rs/target/debug/codex exec --model claude-haiku-4-5 "your prompt"

# interactive TUI
source .env && OPENCODE_API_KEY=$OPENCODE_API_KEY \
  ./codex-rs/target/debug/codex
```

## zenctl.py — all commands

```
./zenctl.py start                  start proxy via supervisord
./zenctl.py stop                   stop proxy + supervisord
./zenctl.py restart                restart just the proxy program
./zenctl.py status                 process state + /health check
./zenctl.py logs [-n N]            last N log lines (default 40)
./zenctl.py models                 list models available via proxy
./zenctl.py test [MODEL]           send "tell me a joke" (default claude-haiku-4-5)
./zenctl.py regen-models           refresh ~/.codex/zen_models.json from live API
```

`zenctl.py` is a self-contained `uv` script — no install, no venv, no PATH fiddling.
`supervisor` is declared as an inline dependency and pulled automatically on first run.

## config.toml (already set up)

`~/.codex/config.toml` points codex at the local proxy:

```toml
model_provider = "zen"

[model_providers.zen]
name            = "Zen"
base_url        = "http://127.0.0.1:9099/v1"
env_key         = "OPENCODE_API_KEY"
wire_api        = "responses"
```

Change `base_url` to `https://opencode.ai/zen/v1` to bypass the proxy entirely
(GPT-only — Claude won't work without the translation layer).

## Using a different provider

The proxy only knows about Zen's endpoints.  To use a different provider:

1. Point `ZEN_BASE_URL` (in `.env`) at the provider's base URL.
2. If the provider speaks native Anthropic Messages API for Claude, the proxy already
   handles the translation.
3. If the provider speaks native OpenAI Responses, set `base_url` in `config.toml`
   directly and skip the proxy.

## Files

| File | What it does |
|------|--------------|
| `zenctl.py` | CLI: start/stop/status/test via supervisord |
| `zen_proxy.py` | The proxy itself (FastAPI + httpx) |
| `zen_models_to_catalog.py` | Converts Zen model list → codex catalog JSON |
| `.env` | API key — keep private |
