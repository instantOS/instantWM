#!/usr/bin/env bash
set -euo pipefail

CAPTURE="${1:-target/profiles/latest}"
if [[ -f "$CAPTURE" ]]; then
  PERF_DATA="$CAPTURE"
  CAPTURE_DIR="$(dirname "$CAPTURE")"
else
  PERF_DATA="$CAPTURE/perf.data"
  CAPTURE_DIR="$CAPTURE"
fi
[[ -f "$PERF_DATA" ]] || { echo "missing $PERF_DATA" >&2; exit 1; }
command -v samply >/dev/null 2>&1 || { echo "samply is not installed" >&2; exit 1; }

pid="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("pid", ""))' "$CAPTURE_DIR/metadata.json" 2>/dev/null || true)"
args=()
[[ -n "$pid" ]] && args+=(--pid "$pid")
samply import --save-only --output "$CAPTURE_DIR/samply-profile.json.gz" "${args[@]}" "$PERF_DATA"
echo "wrote $CAPTURE_DIR/samply-profile.json.gz"
