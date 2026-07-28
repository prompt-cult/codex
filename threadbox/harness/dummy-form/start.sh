#!/bin/sh
# Start the dummy CRM form server in the background and wait for
# readiness. Usage: ./start.sh [port]
set -e
DIR="$(cd "$(dirname "$0")" && pwd)"
PORT="${1:-3456}"

node "$DIR/server.js" "$PORT" > "$DIR/.server.log" 2>&1 &
echo $! > "$DIR/.server.pid"

for i in $(seq 1 40); do
  if curl -sf "http://localhost:$PORT/health" > /dev/null 2>&1; then
    echo "dummy-form server ready on port $PORT (pid $(cat "$DIR/.server.pid"))"
    exit 0
  fi
  sleep 0.25
done

echo "dummy-form server failed to start; see $DIR/.server.log" >&2
kill "$(cat "$DIR/.server.pid")" 2>/dev/null || true
exit 1
