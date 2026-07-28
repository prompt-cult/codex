#!/bin/sh
# Stop the dummy CRM form server started by start.sh.
DIR="$(cd "$(dirname "$0")" && pwd)"
if [ -f "$DIR/.server.pid" ]; then
  kill "$(cat "$DIR/.server.pid")" 2>/dev/null || true
  rm -f "$DIR/.server.pid"
fi
