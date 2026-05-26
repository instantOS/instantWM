#!/usr/bin/env bash
# Print the package version from Cargo.toml to stdout.
#
# If $GITHUB_OUTPUT is set, also writes `version=<v>` to it so the script can
# be used directly as a `run:` step that feeds a later `${{ steps.X.outputs.version }}`.
#
# Optional first argument: a tag ref name (typically $GITHUB_REF_NAME). When
# provided and it looks like a version tag (`v<X.Y.Z>` or `instantwm-v<X.Y.Z>`),
# the version embedded in the tag is checked against Cargo.toml and a mismatch
# is a hard error. Non-tag refs are ignored.
set -euo pipefail

ref_name="${1:-}"

version="$(awk -F '"' '/^version =/ {print $2; exit}' Cargo.toml)"
if [[ -z "$version" ]]; then
  echo "Failed to determine version from Cargo.toml" >&2
  exit 1
fi

if [[ -n "$ref_name" ]]; then
  case "$ref_name" in
    v*)             tag_version="${ref_name#v}" ;;
    instantwm-v*)   tag_version="${ref_name#instantwm-v}" ;;
    *)              tag_version="" ;;
  esac
  if [[ -n "$tag_version" && "$tag_version" != "$version" ]]; then
    echo "Tag version $tag_version does not match Cargo.toml version $version" >&2
    exit 1
  fi
fi

echo "$version"
if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  echo "version=$version" >> "$GITHUB_OUTPUT"
fi
