#!/bin/sh
# shellcheck disable=SC2012,SC1090
# regenerate_zen_models.sh
#
# Fetches the live model list from OpenCode Zen and generates a codex-rs
# ModelInfo catalog JSON at ~/.codex/zen_models.json.
#
# Run this whenever the zen model list changes (infrequently).
# After running, add this line to ~/.codex/config.toml to activate:
#
#   model_catalog_json = "$HOME/.codex/zen_models.json"
#
# Remove that line to go back to the built-in GPT-only catalog.
#
# Usage:
#   ./regenerate_zen_models.sh              # writes to ~/.codex/zen_models.json
#   ./regenerate_zen_models.sh /tmp/test.json  # write to custom path

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ENV_FILE="$SCRIPT_DIR/.env"
OUTPUT="${1:-$HOME/.codex/zen_models.json}"

_check_env_file() {
    env_file="$1"
    if [ ! -f "$env_file" ]; then
        return 0
    fi

    # Check permissions (must be 600 = -rw-------)
    perms=$(ls -l "$env_file" 2>/dev/null | awk '{print $1}' | cut -c1-10)
    case "$perms" in
        -rw-------)
            ;;
        *)
            echo "ERROR: $env_file has permissions $perms — must be -rw------- (600)" >&2
            echo "Fix: chmod 600 $env_file" >&2
            exit 1
            ;;
    esac

    # Check .env is gitignored
    if [ -d "$SCRIPT_DIR/.git" ] || (cd "$SCRIPT_DIR" && git rev-parse --is-inside-work-tree >/dev/null 2>&1); then
        if ! (cd "$SCRIPT_DIR" && git check-ignore -q "$env_file" 2>/dev/null); then
            echo "ERROR: $env_file is NOT ignored by git and may be committed!" >&2
            echo "Fix: echo '.env' >> $SCRIPT_DIR/.gitignore; git rm --cached $env_file 2>/dev/null || true" >&2
            exit 1
        fi
    fi
    return 0
}

_check_env_file "$ENV_FILE"

# Load .env
if [ -f "$ENV_FILE" ]; then
    set -a
    . "$ENV_FILE"
    set +a
fi

if [ -z "${OPENCODE_API_KEY:-}" ]; then
    echo "ERROR: OPENCODE_API_KEY not set. Source .env or export it." >&2
    exit 1
fi

echo "Fetching models from OpenCode Zen..."
MODELS_JSON=$(curl -sf "https://opencode.ai/zen/v1/models" \
    -H "Authorization: Bearer $OPENCODE_API_KEY" \
    -H "Content-Type: application/json")

if [ -z "$MODELS_JSON" ]; then
    echo "ERROR: Empty response from zen /models endpoint." >&2
    exit 1
fi

MODEL_COUNT=$(echo "$MODELS_JSON" | python3 -c "import sys,json; print(len(json.load(sys.stdin)['data']))")
echo "Found $MODEL_COUNT models. Generating catalog..."

python3 "$SCRIPT_DIR/zen_models_to_catalog.py" <<EOF > "$OUTPUT"
$MODELS_JSON
EOF

WRITTEN=$(python3 -c "import sys,json; d=json.load(open('$OUTPUT')); print(len(d['models']))")
echo "Written $WRITTEN model entries to $OUTPUT"
echo ""
echo "To activate, add this line to ~/.codex/config.toml:"
echo "  model_catalog_json = \"$OUTPUT\""
echo ""
echo "To deactivate (return to built-in GPT catalog), remove that line."