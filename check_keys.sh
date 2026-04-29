#!/bin/bash
# Read .env file and check which API keys are set
if [ -f /Users/Shared/codex/.env ]; then
    set -a
    source /Users/Shared/codex/.env
    set +a
fi

echo "OPENCODE_API_KEY set: ${OPENCODE_API_KEY:+yes}"
echo "ZEN_API_KEY set: ${ZEN_API_KEY:+yes}"
echo "OPENAI_API_KEY set: ${OPENAI_API_KEY:+yes}"