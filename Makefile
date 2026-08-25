# Process wrapper for side-by-side testing of the translating proxies
# (stable `zen-proxy` reference vs experimental uncommitted `mistral-proxy`).
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
# BIN points at a version-pinned copy of the release `codex` binary so that
# concurrent source-tree changes cannot affect a running test series.

SHELL := /bin/bash
.SHELLFLAGS := -eu -o pipefail -c

BIN      ?= .proxy-test-bins/$(shell git rev-parse --short HEAD 2>/dev/null || echo unknown)/codex
ENV_FILE ?= .env
RUN_DIR  ?= .tmp/proxy-test

ZEN_MODEL     ?= gpt-5.6-luna
MISTRAL_MODEL ?= mistral-medium-latest

CURL := curl -fsS --max-time 30

.PHONY: port boot-zen boot-mistral smoke smoke-zen smoke-mistral stop-all clean help

help:
	@sed -n '2,20p' $(firstword $(MAKEFILE_LIST))

port:
	@./scripts/free-high-port.sh

boot-zen:
	@KEY_ENV=OPENCODE_API_KEY PROXY=zen MODEL='$(ZEN_MODEL)' \
		./scripts/boot-proxy.sh '$(BIN)' '$(ENV_FILE)' '$(RUN_DIR)'

boot-mistral:
	@KEY_ENV=MISTRAL_API_KEY PROXY=mistral MODEL='$(MISTRAL_MODEL)' \
		./scripts/boot-proxy.sh '$(BIN)' '$(ENV_FILE)' '$(RUN_DIR)'

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
