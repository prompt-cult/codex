#!/usr/bin/env bash
# start_zen_proxy.sh — start, stop, restart, or check status of zen_proxy
#
# Usage:
#   ./start_zen_proxy.sh start    — start proxy in background
#   ./start_zen_proxy.sh stop     — stop running proxy
#   ./start_zen_proxy.sh restart  — stop then start
#   ./start_zen_proxy.sh status   — show pid and health
#   ./start_zen_proxy.sh logs     — tail the log file
#   ./start_zen_proxy.sh fg       — run in foreground (for debugging)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROXY_SCRIPT="$SCRIPT_DIR/zen_proxy.py"
PID_FILE="/tmp/zen_proxy.pid"
LOG_FILE="/tmp/zen_proxy.log"
ENV_FILE="$SCRIPT_DIR/.env"
PORT="${ZEN_PROXY_PORT:-9099}"

# Load env
if [[ -f "$ENV_FILE" ]]; then
    set -a
    # shellcheck disable=SC1090
    source "$ENV_FILE"
    set +a
fi

_pid_running() {
    [[ -f "$PID_FILE" ]] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null
}

cmd="${1:-status}"

case "$cmd" in
    start)
        if _pid_running; then
            echo "zen_proxy already running (pid=$(cat "$PID_FILE"))"
            exit 0
        fi
        echo "Starting zen_proxy on port $PORT ..."
        echo "Logs: $LOG_FILE"
        nohup "$PROXY_SCRIPT" >> "$LOG_FILE" 2>&1 &
        BGPID=$!
        echo "$BGPID" > "$PID_FILE"
        # give it a moment then health-check
        sleep 2
        if kill -0 "$BGPID" 2>/dev/null; then
            HEALTH=$(curl -sf "http://127.0.0.1:$PORT/health" 2>/dev/null || echo '{"status":"unreachable"}')
            echo "zen_proxy started pid=$BGPID health=$HEALTH"
        else
            echo "ERROR: zen_proxy failed to start — check $LOG_FILE"
            tail -30 "$LOG_FILE"
            exit 1
        fi
        ;;

    stop)
        if _pid_running; then
            PID=$(cat "$PID_FILE")
            echo "Stopping zen_proxy pid=$PID ..."
            kill "$PID"
            # wait up to 5s
            for _ in $(seq 1 10); do
                kill -0 "$PID" 2>/dev/null || break
                sleep 0.5
            done
            kill -0 "$PID" 2>/dev/null && kill -9 "$PID" || true
            rm -f "$PID_FILE"
            echo "zen_proxy stopped"
        else
            echo "zen_proxy not running"
        fi
        ;;

    restart)
        "$0" stop || true
        sleep 1
        "$0" start
        ;;

    status)
        if _pid_running; then
            PID=$(cat "$PID_FILE")
            HEALTH=$(curl -sf "http://127.0.0.1:$PORT/health" 2>/dev/null || echo '{"status":"unreachable"}')
            echo "zen_proxy RUNNING pid=$PID port=$PORT health=$HEALTH"
        else
            echo "zen_proxy STOPPED"
        fi
        ;;

    logs)
        tail -f "$LOG_FILE"
        ;;

    fg)
        echo "Starting zen_proxy in foreground on port $PORT ..."
        exec "$PROXY_SCRIPT"
        ;;

    test-mini)
        echo "=== Testing gpt-5.4-mini via proxy (streaming) ==="
        curl -s "http://127.0.0.1:$PORT/v1/responses" \
            -H "Authorization: Bearer ${OPENCODE_API_KEY:-missing}" \
            -H "Content-Type: application/json" \
            -d '{"model":"gpt-5.4-mini","stream":true,"input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"tell me a joke"}]}]}' \
            | grep -E "^(event:|data:)" | head -40
        ;;

    test-haiku)
        echo "=== Testing claude-haiku-4-5 via proxy (streaming, SSE normalisation) ==="
        curl -s "http://127.0.0.1:$PORT/v1/responses" \
            -H "Authorization: Bearer ${OPENCODE_API_KEY:-missing}" \
            -H "Content-Type: application/json" \
            -d '{"model":"claude-haiku-4-5","stream":true,"input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"tell me a joke"}]}]}' \
            | grep -E "^(event:|data:)" | head -40
        ;;

    test-haiku-nonstream)
        echo "=== Testing claude-haiku-4-5 via proxy (non-streaming) ==="
        curl -s "http://127.0.0.1:$PORT/v1/responses" \
            -H "Authorization: Bearer ${OPENCODE_API_KEY:-missing}" \
            -H "Content-Type: application/json" \
            -d '{"model":"claude-haiku-4-5","stream":false,"input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"tell me a joke"}]}]}' \
            | jq '{status: (if .error then "ERROR" else "OK" end), error: .error, text: .output[0].content[0].text}'
        ;;

    test-models)
        echo "=== Testing /v1/models via proxy ==="
        curl -sf "http://127.0.0.1:$PORT/v1/models" \
            -H "Authorization: Bearer ${OPENCODE_API_KEY:-missing}" \
            | jq '[.data[].id]'
        ;;

    *)
        echo "Usage: $0 {start|stop|restart|status|logs|fg|test-mini|test-haiku|test-haiku-nonstream|test-models}"
        exit 1
        ;;
esac
