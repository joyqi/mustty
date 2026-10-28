#!/usr/bin/env bash
# Fails unless the release tag is exactly v<version from Cargo.toml>.
# Usage: check-tag.sh <tag>
set -euo pipefail

tag="${1:?usage: check-tag.sh <tag>}"
version=$(cargo metadata --no-deps --format-version 1 \
  | jq -r '.packages[] | select(.name == "mustty") | .version')

if [[ "$tag" != v* ]]; then
  msg="Tag '$tag' is not a release tag: release tags are v<version>, here 'v$version'."
elif [ "$tag" != "v$version" ]; then
  msg="Tag '$tag' does not match Cargo.toml version '$version'. Either push tag 'v$version', or set version = \"${tag#v}\" in Cargo.toml, commit, and tag that commit."
fi
if [ -n "${msg:-}" ]; then
  echo "::error title=Release tag does not match Cargo.toml::$msg"
  exit 1
fi
echo "Tag $tag matches Cargo.toml version $version"
