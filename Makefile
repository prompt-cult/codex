# ── toolchain bootstrap ───────────────────────────────────────────────────────
# Use uv directly if it's already on PATH (vanilla install, company WSL, CI).
# Fall back to mise only when uv is absent — no ~/.zshrc required, no global
# shell activation, works identically on macOS zsh and WSL bash.
#
# On a blank machine:
#   curl https://mise.run | sh && ~/.local/bin/mise install && make init
# On a machine that already has uv:
#   make init

UV := $(shell command -v uv 2>/dev/null)

ifndef UV
  MISE := $(shell command -v mise 2>/dev/null || echo ~/.local/bin/mise)
  UV   := $(shell $(MISE) exec -- command -v uv 2>/dev/null)
  ifeq ($(UV),)
    $(error uv not found. Run: curl https://mise.run | sh && ~/.local/bin/mise install)
  endif
  RUN := $(MISE) exec --
else
  RUN :=
endif

# ── targets ───────────────────────────────────────────────────────────────────

.PHONY: init
## Bootstrap: install mise tools if needed, then install Python deps
init:
	@if ! command -v mise >/dev/null 2>&1 && [ ! -x ~/.local/bin/mise ]; then \
	  echo "Installing mise..."; \
	  curl https://mise.run | sh; \
	fi
	@if command -v mise >/dev/null 2>&1 || [ -x ~/.local/bin/mise ]; then \
	  $$(command -v mise || echo ~/.local/bin/mise) install --quiet; \
	fi
	@echo "uv: $$($(RUN) uv --version)"
	@echo "python: $$($(RUN) uv run python --version)"

.PHONY: start
## Start the zen proxy
start:
	$(RUN) ./zenctl.py start

.PHONY: stop
stop:
	$(RUN) ./zenctl.py stop

.PHONY: status
status:
	$(RUN) ./zenctl.py status

.PHONY: test-proxy
## Smoke-test the proxy with a single LLM call
test-proxy:
	$(RUN) ./zenctl.py test
