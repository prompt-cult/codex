# Process wrapper for side-by-side testing of the translating proxies
# (`codex-zen-proxy` stable reference vs `codex-mistral-proxy`).
#
# The proxies never exit, so every `boot-*` target backgrounds the process,
# records its pid/server-info/log under .tmp/proxy-test/, and waits (max 30s)
# for its /health endpoint before returning. All network commands use curl
# --max-time so nothing can hang the harness. Ports default to a free high
# port picked per invocation; override with PORT=.
#
#   make boot-zen                 # OPENCODE_API_KEY from .env
#   make boot-mistral             # MISTRAL_API_KEY from .env
#   make smoke                    # health + models + non-stream + stream on both
#   make stop-all
#
# The workspace is slimmed to the proxy crates only; each proxy is its own
# standalone binary (no `codex` multitool subcommand anymore).

SHELL := /bin/bash
.SHELLFLAGS := -eu -o pipefail -c

BIN_ZEN     ?= codex-rs/target/release/codex-zen-proxy
BIN_MISTRAL ?= codex-rs/target/release/codex-mistral-proxy

ENV_FILE ?= .env
RUN_DIR  ?= .tmp/proxy-test

ZEN_MODEL     ?= gpt-5.6-luna
MISTRAL_MODEL ?= mistral-medium-latest

.PHONY: port boot-zen boot-mistral smoke smoke-zen smoke-mistral stop-all clean help

help:
	@sed -n '2,20p' $(firstword $(MAKEFILE_LIST))

port:
	@./scripts/free-high-port.sh

boot-zen:
	@KEY_ENV=OPENCODE_API_KEY PROXY=zen MODEL='$(ZEN_MODEL)' \
		./scripts/boot-proxy.sh '$(BIN_ZEN)' '$(ENV_FILE)' '$(RUN_DIR)'

boot-mistral:
	@KEY_ENV=MISTRAL_API_KEY PROXY=mistral MODEL='$(MISTRAL_MODEL)' \
		./scripts/boot-proxy.sh '$(BIN_MISTRAL)' '$(ENV_FILE)' '$(RUN_DIR)'

smoke: smoke-zen smoke-mistral

smoke-zen:
	@PROXY=zen MODEL='$(ZEN_MODEL)' RUN_DIR='$(RUN_DIR)' ./scripts/smoke-proxy.sh

smoke-mistral:
	@PROXY=mistral MODEL='$(MISTRAL_MODEL)' RUN_DIR='$(RUN_DIR)' ./scripts/smoke-proxy.sh

stop-all:
	@for pid in $(RUN_DIR)/*.pid; do \
		[ -f "$$pid" ] && kill "$$(cat $$pid)" 2>/dev/null || true; \
	done
	@echo "stopped all proxies"

clean: stop-all
	rm -rf $(RUN_DIR)
