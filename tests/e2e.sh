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
  run_ctl geom "$id" >>/tmp/iwm-e2e-geoms.txt
done </tmp/iwm-e2e-ids-new.txt

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

echo "PASS: spawned=$new_count"
