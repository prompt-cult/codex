#!/usr/bin/env bash
# Boot one translating proxy in the background and wait (max 30s) for health.
#
# Env:
#   PORT      listen port (required; caller usually picks a free high port)
#   KEY_ENV   variable name in ENV_FILE holding the upstream API key
#   PROXY     subcommand name: zen | mistral
# Args:
#   $1 codex binary, $2 env file, $3 run dir
set -euo pipefail

bin="${1:?usage: boot-proxy.sh BIN ENV_FILE RUN_DIR}"
env_file="${2:?}"
run_dir="${3:?}"
script_dir="$(cd "$(dirname "$0")" && pwd)"
port="${PORT:-$("$script_dir/free-high-port.sh")}"
key_env="${KEY_ENV:?KEY_ENV required}"
proxy="${PROXY:?PROXY required}"

pid_file="$run_dir/$proxy.pid"
log_file="$run_dir/$proxy.log"
info_file="$run_dir/$proxy.json"

mkdir -p "$run_dir"

key="$(awk -F= -v k="export $key_env" '$1 == k { sub(/^[^=]*=/, "", $0); print; exit }' "$env_file")"
if [ -z "$key" ]; then
  echo "error: $key_env not found in $env_file" >&2
  exit 1
fi

# Fresh state per boot.
if [ -f "$pid_file" ] && kill -0 "$(cat "$pid_file")" 2>/dev/null; then
  echo "error: $proxy proxy already running (pid $(cat "$pid_file"))" >&2
  exit 1
fi
rm -f "$pid_file" "$info_file" "$log_file"

printf '%s' "$key" | nohup "$bin" "$proxy-proxy" \
  --port "$port" --http-shutdown --server-info "$info_file" \
  >"$log_file" 2>&1 &
echo $! >"$pid_file"
unset key

# Wait up to 30s for /health.
deadline=$(( $(date +%s) + 30 ))
until curl -fsS --max-time 2 "http://127.0.0.1:$port/health" >/dev/null 2>&1; do
  if ! kill -0 "$(cat "$pid_file")" 2>/dev/null; then
    echo "error: $proxy proxy exited during startup; log follows:" >&2
    cat "$log_file" >&2 || true
    exit 1
  fi
  if [ "$(date +%s)" -ge "$deadline" ]; then
    echo "error: $proxy proxy not healthy after 30s; log follows:" >&2
    cat "$log_file" >&2 || true
    exit 1
  fi
  sleep 0.5
done

echo "$proxy proxy: http://127.0.0.1:$port (pid $(cat "$pid_file"), info $info_file)"
