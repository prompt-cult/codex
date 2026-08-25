#!/usr/bin/env bash
# Run the same four probes against one booted proxy.
#
# Env: PROXY (zen|mistral), MODEL (slug to ask for), RUN_DIR
# Reads <RUN_DIR>/<PROXY>.json for {"port":...}.
set -uo pipefail

proxy="${PROXY:?PROXY required}"
model="${MODEL:?MODEL required}"
run_dir="${RUN_DIR:?RUN_DIR required}"

info="$run_dir/$proxy.json"
[ -f "$info" ] || { echo "error: $info missing (boot $proxy first)" >&2; exit 1; }
port="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["port"])' "$info")"
base="http://127.0.0.1:$port"

fail=0
step() { printf '\n=== [%s] %s ===\n' "$proxy" "$1"; }

step "health"
curl -fsS --max-time 10 "$base/health" && echo || fail=1

step "models (GET /v1/models?client_version=0.99.0)"
# Query param included deliberately: codex always sends it, and exact-match
# routing that ignores query strings 403s here.
curl -sS --max-time 30 "$base/v1/models?client_version=0.99.0" | head -c 600 && echo \
  || { echo "FAILED models probe"; fail=1; }

step "non-stream prompt ($model)"
resp="$(curl -sS --max-time 30 -X POST "$base/v1/responses" \
  -H 'content-type: application/json' \
  -d "{\"model\":\"$model\",\"stream\":false,\"input\":[{\"type\":\"message\",\"role\":\"user\",\"content\":\"Reply with exactly: PROXY OK\"}]}")"
echo "${resp:0:800}"
case "$resp" in *'"status":"completed"'*) : ;; *) echo "FAILED non-stream probe"; fail=1 ;; esac

step "stream prompt ($model) — first translated SSE events"
curl -sN --max-time 30 -X POST "$base/v1/responses" \
  -H 'content-type: application/json' \
  -d "{\"model\":\"$model\",\"stream\":true,\"input\":[{\"type\":\"message\",\"role\":\"user\",\"content\":\"Count from one to three, digits only.\"}]}" \
  | head -n 40

if [ "$fail" -ne 0 ]; then
  echo "SMOKE FAILED for $proxy" >&2
  exit 1
fi
echo "SMOKE PASSED for $proxy"
