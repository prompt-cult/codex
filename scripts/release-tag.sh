#!/usr/bin/env bash
set -euo pipefail

# Get today's date in YYYY.MM.DD format
DATE=$(date +%Y.%m.%d)

# Get the short SHA of the current commit
SHA=$(git rev-parse --short=10 HEAD)

# Surface the local checkout state in the computed version string.
DIRTY_SUFFIX=""
if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
  DIRTY_SUFFIX="-dirty"
fi

# Construct the tag name
TAG="${DATE}-${SHA}${DIRTY_SUFFIX}"

# Dirty local trees are not reproducible release points, so refuse to tag them.
if [[ -n "${DIRTY_SUFFIX}" ]]; then
  echo "❌ Error: Working directory is dirty. Refusing to create non-reproducible tag ${TAG}."
  git status --short
  exit 1
fi

# Create the annotated tag
echo "Creating tag: ${TAG}"
git tag -a "${TAG}" -m "Release ${TAG}"

# Push the tag to origin
echo "Pushing tag to origin..."
git push origin "${TAG}"

echo "Done! CI build triggered for ${TAG}."
