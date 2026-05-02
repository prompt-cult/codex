#!/bin/sh
# shellcheck disable=SC2012,SC1090
# check_keys.sh — verify API keys are configured

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ENV_FILE="$SCRIPT_DIR/.env"

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
            return 1
            ;;
    esac

    # Check .env is gitignored
    if [ -d "$SCRIPT_DIR/.git" ] || (cd "$SCRIPT_DIR" && git rev-parse --is-inside-work-tree >/dev/null 2>&1); then
        if ! (cd "$SCRIPT_DIR" && git check-ignore -q "$env_file" 2>/dev/null); then
            echo "ERROR: $env_file is NOT ignored by git and may be committed!" >&2
            echo "Fix: echo '.env' >> $SCRIPT_DIR/.gitignore; git rm --cached $env_file 2>/dev/null || true" >&2
            return 1
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

if [ -n "${OPENCODE_API_KEY:-}" ]; then
    echo "OPENCODE_API_KEY set: yes"
else
    echo "OPENCODE_API_KEY set: no"
fi

if [ -n "${ZEN_API_KEY:-}" ]; then
    echo "ZEN_API_KEY set: yes"
else
    echo "ZEN_API_KEY set: no"
fi

if [ -n "${OPENAI_API_KEY:-}" ]; then
    echo "OPENAI_API_KEY set: yes"
else
    echo "OPENAI_API_KEY set: no"
fi