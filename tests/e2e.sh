#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUST_DIR="$ROOT_DIR/rust"
SOCKET_PATH="${INSTANTWM_SOCKET:-/tmp/instantwm-$(id -u).sock}"
WM_LOG="${WM_LOG:-/tmp/instantwm-e2e-wm.log}"
SPAWN_COUNT="${SPAWN_COUNT:-3}"
STARTUP_RETRIES="${STARTUP_RETRIES:-200}"
STARTUP_SLEEP="${STARTUP_SLEEP:-0.1}"

choose_spawn_cmd() {
  local idx="$1"
  if [[ -n "${E2E_APP_CMD:-}" ]]; then
    printf '%s' "$E2E_APP_CMD"
    return 0
  fi
  if command -v gtk3-demo >/dev/null 2>&1; then
    printf 'gtk3-demo'
    return 0
  fi
  if command -v xmessage >/dev/null 2>&1; then
    printf 'xmessage -name iwm-e2e-%s iwm-e2e-%s' "$idx" "$idx"
    return 0
  fi
  echo "No supported test app found (need gtk3-demo or xmessage)" >&2
  return 1
}

run_ctl() {
  "$RUST_DIR/target/debug/instantwmctl" "$@"
}

cleanup() {
  if [[ -n "${wm_pid:-}" ]]; then
    kill "$wm_pid" >/dev/null 2>&1 || true
    wait "$wm_pid" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

cd "$RUST_DIR"
cargo build --quiet --bin instantwm --bin instantwmctl

rm -f "$SOCKET_PATH" "$WM_LOG" /tmp/iwm-e2e-list-* /tmp/iwm-e2e-ids-* /tmp/iwm-e2e-geoms.txt
INSTANTWM_AUTOSTART=0 INSTANTWM_WL_AUTOSPAWN=0 timeout 45s ./target/debug/instantwm --backend wayland >"$WM_LOG" 2>&1 &
wm_pid=$!

for _ in $(seq 1 "$STARTUP_RETRIES"); do
  if run_ctl list >/tmp/iwm-e2e-list-initial.txt 2>/dev/null; then
    break
  fi
  sleep "$STARTUP_SLEEP"
done

run_ctl list >/tmp/iwm-e2e-list-initial.txt
awk 'NR>1 && $1 ~ /^[0-9]+$/ {print $1}' /tmp/iwm-e2e-list-initial.txt | sort -n >/tmp/iwm-e2e-ids-initial.txt

for idx in $(seq 1 "$SPAWN_COUNT"); do
  cmd="$(choose_spawn_cmd "$idx")"
  run_ctl spawn "$cmd" >/tmp/iwm-e2e-spawn-"$idx".txt
done

sleep 2
for _ in $(seq 1 50); do
  run_ctl list >/tmp/iwm-e2e-list-after-spawn.txt
  awk 'NR>1 && $1 ~ /^[0-9]+$/ {print $1}' /tmp/iwm-e2e-list-after-spawn.txt | sort -n >/tmp/iwm-e2e-ids-after.txt
  comm -13 /tmp/iwm-e2e-ids-initial.txt /tmp/iwm-e2e-ids-after.txt >/tmp/iwm-e2e-ids-new.txt
  new_count="$(wc -l </tmp/iwm-e2e-ids-new.txt | tr -d ' ')"
  if (( new_count >= SPAWN_COUNT )); then
    break
  fi
  sleep 0.1
done

new_count="$(wc -l </tmp/iwm-e2e-ids-new.txt | tr -d ' ')"
if (( new_count < SPAWN_COUNT )); then
  echo "Expected at least $SPAWN_COUNT new windows, got $new_count" >&2
  exit 1
fi

: >/tmp/iwm-e2e-geoms.txt
while read -r id; do
  run_ctl --json window info "$id" | python3 -c '
import json, sys
w = json.load(sys.stdin)
g = w["geometry"]
print(w["id"], g["x"], g["y"], g["width"], g["height"])
' >>/tmp/iwm-e2e-geoms.txt
done </tmp/iwm-e2e-ids-new.txt

awk 'NF >= 5 && $1 ~ /^[0-9]+$/ {print $0}' /tmp/iwm-e2e-geoms.txt >/tmp/iwm-e2e-geoms-clean.txt
if ! awk 'NF >= 5 && $3 > 0 {found=1} END{exit(found?0:1)}' /tmp/iwm-e2e-geoms-clean.txt; then
  echo "No window y-offset detected; bar may be overlaying tiled area" >&2
  exit 1
fi
if ! awk -v expected="$SPAWN_COUNT" '
BEGIN { n=0; bad=0; max_ratio=0.0 }
{
  n++;
  id[n]=$1; x[n]=$2; y[n]=$3; w[n]=$4; h[n]=$5;
  if (w[n] <= 0 || h[n] <= 0) bad=1;
}
END {
  if (n < expected) {
    print "Missing geometry rows: expected at least " expected ", got " n > "/dev/stderr";
    exit 1;
  }
  for (i = 1; i <= n; i++) {
    for (j = i + 1; j <= n; j++) {
      left = (x[i] > x[j]) ? x[i] : x[j];
      top = (y[i] > y[j]) ? y[i] : y[j];
      right = ((x[i] + w[i]) < (x[j] + w[j])) ? (x[i] + w[i]) : (x[j] + w[j]);
      bottom = ((y[i] + h[i]) < (y[j] + h[j])) ? (y[i] + h[i]) : (y[j] + h[j]);
      ow = right - left;
      oh = bottom - top;
      if (ow < 0) ow = 0;
      if (oh < 0) oh = 0;
      overlap = ow * oh;
      a1 = w[i] * h[i];
      a2 = w[j] * h[j];
      base = (a1 < a2) ? a1 : a2;
      ratio = (base > 0) ? overlap / base : 1.0;
      if (ratio > max_ratio) max_ratio = ratio;
      if (ratio > 0.70) {
        print "Excessive overlap between windows " id[i] " and " id[j] ": " ratio > "/dev/stderr";
        bad = 1;
      }
    }
  }
  if (bad) exit 1;
  print "max_overlap_ratio=" max_ratio > "/tmp/iwm-e2e-overlap.txt";
}
' /tmp/iwm-e2e-geoms-clean.txt; then
  echo "Geometry sanity check failed; see /tmp/iwm-e2e-geoms-clean.txt" >&2
  exit 1
fi

while read -r id; do
  run_ctl close "$id" >/dev/null
done </tmp/iwm-e2e-ids-new.txt

sleep 1
run_ctl list >/tmp/iwm-e2e-list-after-close.txt
awk 'NR>1 && $1 ~ /^[0-9]+$/ {print $1}' /tmp/iwm-e2e-list-after-close.txt | sort -n >/tmp/iwm-e2e-ids-final.txt

if ! diff -u /tmp/iwm-e2e-ids-initial.txt /tmp/iwm-e2e-ids-final.txt >/tmp/iwm-e2e-id-diff.txt; then
  echo "Window IDs mismatch after close; see /tmp/iwm-e2e-id-diff.txt" >&2
  exit 1
fi

if grep -Eq "panic|negative size|libEGL warning|Gdk-CRITICAL" "$WM_LOG"; then
  echo "WM log contains critical errors; see $WM_LOG" >&2
  exit 1
fi

echo "PASS: spawned=$new_count $(cat /tmp/iwm-e2e-overlap.txt)"
