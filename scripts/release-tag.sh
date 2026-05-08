#!/usr/bin/env bash
set -euo pipefail

# Get today's date in YYYY.MM.DD format
DATE=$(date +%Y.%m.%d)

# Get the short SHA of the current commit
SHA=$(git rev-parse --short HEAD)

# Construct the tag name
TAG="${DATE}-${SHA}"

# Create the annotated tag
echo "Creating tag: ${TAG}"
git tag -a "${TAG}" -m "Release ${TAG}"

# Push the tag to origin
echo "Pushing tag to origin..."
git push origin "${TAG}"

echo "Done! CI build triggered for ${TAG}."
