#!/usr/bin/env bash
# regenerate_zen_models.sh
#
# Fetches the live model list from OpenCode Zen and generates a codex-rs
# ModelInfo catalog JSON at ~/.codex/zen_models.json.
#
# Run this whenever the zen model list changes (infrequently).
# After running, add this line to ~/.codex/config.toml to activate:
#
#   model_catalog_json = "/Users/$USER/.codex/zen_models.json"
#
# Remove that line to go back to the built-in GPT-only catalog.
#
# Usage:
#   ./regenerate_zen_models.sh              # writes to ~/.codex/zen_models.json
#   ./regenerate_zen_models.sh /tmp/test.json  # write to custom path

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENV_FILE="$SCRIPT_DIR/.env"
OUTPUT="${1:-$HOME/.codex/zen_models.json}"

# Load .env
if [[ -f "$ENV_FILE" ]]; then
    set -a
    # shellcheck disable=SC1090
    source "$ENV_FILE"
    set +a
fi

if [[ -z "${OPENCODE_API_KEY:-}" ]]; then
    echo "ERROR: OPENCODE_API_KEY not set. Source .env or export it." >&2
    exit 1
fi

echo "Fetching models from OpenCode Zen..."
MODELS_JSON=$(curl -sf "https://opencode.ai/zen/v1/models" \
    -H "Authorization: Bearer $OPENCODE_API_KEY" \
    -H "Content-Type: application/json")

if [[ -z "$MODELS_JSON" ]]; then
    echo "ERROR: Empty response from zen /models endpoint." >&2
    exit 1
fi

MODEL_COUNT=$(echo "$MODELS_JSON" | python3 -c "import sys,json; print(len(json.load(sys.stdin)['data']))")
echo "Found $MODEL_COUNT models. Generating catalog..."

python3 "$SCRIPT_DIR/zen_models_to_catalog.py" <<< "$MODELS_JSON" > "$OUTPUT"

WRITTEN=$(python3 -c "import sys,json; d=json.load(open('$OUTPUT')); print(len(d['models']))")
echo "Written $WRITTEN model entries to $OUTPUT"
echo ""
echo "To activate, add this line to ~/.codex/config.toml:"
echo "  model_catalog_json = \"$OUTPUT\""
echo ""
echo "To deactivate (return to built-in GPT catalog), remove that line."
