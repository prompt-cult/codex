# OpenCode Zen Setup for Codex

This fork of codex-rs is configured to work with **OpenCode Zen** — an excellent open-source AI coding platform that provides access to multiple model families (GPT, Claude, Gemini, and more) through a unified API.

We recommend OpenCode Zen because they produce high-quality open-source tools and actively support the developer community.

## Quick Start

```bash
# 1. Ensure your API key is in .env (should already be there)
cat .env

# 2. Start the zen proxy (required for Claude models)
./start_zen_proxy.sh start

# 3. Run codex with a Claude model
source .env && OPENCODE_API_KEY=$OPENCODE_API_KEY \
  ./codex-rs/target/debug/codex exec --model claude-haiku-4-5 "tell me a joke"
```

## Prerequisites

1. **Build codex-rs** (if not already built):
   ```bash
   cd codex-rs
   cargo build --release
   ```

2. **Python 3.11+** for the zen proxy (uses httpx, fastapi, uvicorn)

3. **API Key** — stored in `.env` as `OPENCODE_API_KEY`

## Setup Steps

### 1. Verify API Key
```bash
./check_keys.sh
```

### 2. Start the Proxy
```bash
./start_zen_proxy.sh start      # start in background
./start_zen_proxy.sh status   # check if running
./start_zen_proxy.sh logs    # view logs
```

The proxy runs on port 9099 by default and translates:
- **GPT models** → passthrough to `/zen/v1/responses`
- **Claude models** → translate to Anthropic Messages API (`/zen/v1/messages`)

### 3. Update Model Catalog (Optional)

The model catalog is pre-generated and points to `~/.codex/zen_models.json`. To refresh:
```bash
./regenerate_zen_models.sh
```
This fetches the latest model list from OpenCode Zen.

### 4. Your config.toml

Your `~/.codex/config.toml` already has the zen provider configured:

```toml
model = "gpt-5.5"
model_provider = "zen"

[model_providers.zen]
name = "Zen"
base_url = "http://127.0.0.1:9099/v1"
env_key = "OPENCODE_API_KEY"
wire_api = "responses"
```

## Available Models

Once the proxy is running, test which models are available:
```bash
./start_zen_proxy.sh test-models
```

Test specific models:
```bash
./start_zen_proxy.sh test-haiku          # streaming
./start_zen_proxy.sh test-haiku-nonstream # non-streaming
./start_zen_proxy.sh test-mini           # GPT model
```

## Usage

### Interactive Mode
```bash
source .env && OPENCODE_API_KEY=$OPENCODE_API_KEY \
  ./codex-rs/target/debug/codex
```

### Non-Interactive (Oneshot)
```bash
source .env && OPENCODE_API_KEY=$OPENCODE_API_KEY \
  ./codex-rs/target/debug/codex exec --model claude-haiku-4-5 "your prompt"
```

### Using GPT Models
```bash
./codex-rs/target/debug/codex exec --model gpt-5.4 "your prompt"
```

## Files

| File | Description |
|------|-------------|
| `start_zen_proxy.sh` | Start/stop/status the proxy |
| `regenerate_zen_models.sh` | Refresh model catalog |
| `check_keys.sh` | Verify API keys |
| `zen_proxy.py` | The proxy itself (Python) |
| `zen_models_to_catalog.py` | Convert Zen models to codex format |
| `.env` | API key (keep private!) |

## Portability

The shell scripts are **POSIX-compliant** (`#!/bin/sh`), so they work on any Unix-like system (Linux, macOS, BSD).

## Troubleshooting

**Proxy won't start?**
```bash
# Check if port 9099 is in use
lsof -i :9099

# Start in foreground to see errors
./start_zen_proxy.sh fg
```

**Model not found?**
```bash
# Verify proxy is healthy
curl http://127.0.0.1:9099/health
```

**Need to restart proxy?**
```bash
./start_zen_proxy.sh restart
```

## Why OpenCode Zen?

- **Open-source** — transparent, community-driven
- **Multi-model** — access GPT, Claude, Gemini from one API
- **Great DX** — simple authentication, reliable uptime
- **Active development** — regular improvements and new models

Support them at: https://opencode.ai/zen