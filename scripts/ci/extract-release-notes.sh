#!/usr/bin/env bash
# Print the CHANGELOG.md section for a given version to stdout.
#
# Usage: extract-release-notes.sh <version>
#
# Captures everything between `## [<version>]` (exclusive of the header line)
# and the next `## [` heading. If the section is empty or missing, falls back
# to a one-line placeholder.
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <version>" >&2
  exit 2
fi
version="$1"

notes="$(awk -v version="$version" '
  $0 ~ "^## \\[" version "\\]" { capture = 1; next }
  capture && $0 ~ "^## \\["    { exit }
  capture                       { print }
' CHANGELOG.md)"

if ! grep -q '[^[:space:]]' <<<"$notes"; then
  printf 'Release v%s\n' "$version"
else
  printf '%s\n' "$notes"
fi
