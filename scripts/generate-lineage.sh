#!/usr/bin/env bash
set -euo pipefail

OUTPUT="README.lineage.md"
TODAY=$(date +%Y-%m-%d)

echo "Generating $OUTPUT..."

# Find the nearest upstream tag in our history
BASE_TAG=$(git describe --tags --match "rust-v[0-9]*" --abbrev=0 HEAD 2>/dev/null || echo "unknown")

# Find the latest upstream tag on any remote (excluding alpha/beta/prerelease)
LATEST_UPSTREAM_TAG=$(git tag -l "rust-v[0-9]*" --sort=-v:refname | grep -E '^rust-v[0-9]+\.[0-9]+\.[0-9]+$' | head -n1 || echo "unknown")

# Common ancestor with upstream main
ANCESTOR=$(git merge-base HEAD upstream/main 2>/dev/null || echo "unknown")

{
  echo "# Lineage Report"
  echo ""
  echo "Generated on: $TODAY"
  echo ""
  
  if [[ "$BASE_TAG" != "unknown" ]]; then
    BASE_DATE=$(git log -1 --format=%cs "$BASE_TAG")
    echo "## Base Upstream Version"
    echo "- **Version**: \`$BASE_TAG\`"
    echo "- **Date**: $BASE_DATE"
  else
    echo "## Base Upstream Version"
    echo "Could not identify a clear upstream base tag in the history."
  fi

  echo ""
  echo "## Upstream Status"
  
  if [[ "$LATEST_UPSTREAM_TAG" != "unknown" ]]; then
    LATEST_DATE=$(git log -1 --format=%cs "$LATEST_UPSTREAM_TAG")
    echo "- **Latest Upstream Release**: \`$LATEST_UPSTREAM_TAG\` ($LATEST_DATE)"
    
    if [[ "$BASE_TAG" != "unknown" && "$BASE_TAG" != "$LATEST_UPSTREAM_TAG" ]]; then
      COMMITS_BEHIND=$(git rev-list --count "${BASE_TAG}..${LATEST_UPSTREAM_TAG}" 2>/dev/null || echo "N/A")
      echo "- **Distance from Latest Release**: $COMMITS_BEHIND commits behind"
    elif [[ "$BASE_TAG" == "$LATEST_UPSTREAM_TAG" ]]; then
      echo "- **Status**: Based on the latest upstream release."
    fi
  fi

  if [[ "$ANCESTOR" != "unknown" ]]; then
    AHEAD_UPSTREAM=$(git rev-list --count "${ANCESTOR}..upstream/main" 2>/dev/null || echo "N/A")
    echo "- **Commits on upstream/main ahead of this branch**: $AHEAD_UPSTREAM"
  fi

  echo ""
  echo "## Summary"
  echo "This release is based on codex as at $BASE_TAG."
  if [[ "${AHEAD_UPSTREAM:-0}" != "0" ]]; then
    echo "It is currently $AHEAD_UPSTREAM commits behind the latest upstream development branch (\`upstream/main\`)."
  fi

} > "$OUTPUT"

echo "✅ Generated $OUTPUT"
