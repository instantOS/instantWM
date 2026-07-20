#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUST_DIR="$ROOT_DIR"
SPAWN_COUNT="${SPAWN_COUNT:-3}"
STARTUP_RETRIES="${STARTUP_RETRIES:-200}"
STARTUP_SLEEP="${STARTUP_SLEEP:-0.1}"
client_pgids=()

die() {
  echo "e2e: $*" >&2
  exit 1
}

assert_eq() {
  local expected="$1"
  local actual="$2"
  local description="$3"
  if [[ "$actual" != "$expected" ]]; then
    die "$description: expected '$expected', got '$actual'"
  fi
}

choose_spawn_cmd() {
  local idx="$1"
  if [[ -n "${E2E_APP_CMD:-}" ]]; then
    printf '%s' "$E2E_APP_CMD"
  elif command -v foot >/dev/null 2>&1; then
    printf 'foot'
  elif command -v weston-terminal >/dev/null 2>&1; then
    printf 'weston-terminal'
  elif command -v gtk4-demo >/dev/null 2>&1; then
    printf 'gtk4-demo'
  elif command -v gtk3-demo >/dev/null 2>&1; then
    printf 'gtk3-demo'
  elif command -v xmessage >/dev/null 2>&1; then
    printf 'xmessage -name iwm-e2e-%s iwm-e2e-%s' "$idx" "$idx"
  else
    die "no supported test app found; set E2E_APP_CMD to a command that opens a window"
  fi
}

run_ctl() {
  INSTANTWM_SOCKET="$SOCKET_PATH" "$RUST_DIR/target/debug/instantwmctl" "$@"
}

write_window_ids() {
  local destination="$1"
  run_ctl --json window list | python3 -c '
import json, sys
windows = json.load(sys.stdin)
for window in sorted(windows, key=lambda item: item["id"]):
    print(window["id"])
' >"$destination"
}

cleanup() {
  local status=$?
  trap - EXIT
  set +e

  for pgid in "${client_pgids[@]}"; do
    kill -TERM -- "-$pgid" >/dev/null 2>&1
  done
  if [[ -n "${wm_pid:-}" ]]; then
    kill -TERM -- "-$wm_pid" >/dev/null 2>&1
    wait "$wm_pid" >/dev/null 2>&1
  fi
  for pgid in "${client_pgids[@]}"; do
    kill -KILL -- "-$pgid" >/dev/null 2>&1
  done

  if (( status == 0 )) && [[ "${E2E_KEEP_TMP:-0}" != 1 ]]; then
    rm -rf "$E2E_TMP"
  else
    echo "e2e: artifacts preserved in $E2E_TMP" >&2
  fi
  exit "$status"
}

[[ -n "${WAYLAND_DISPLAY:-}" ]] || die "WAYLAND_DISPLAY is not set; run this nested test from a Wayland session"
command -v setsid >/dev/null 2>&1 || die "setsid is required to isolate and clean up the nested compositor"
[[ "$SPAWN_COUNT" =~ ^[1-9][0-9]*$ ]] || die "SPAWN_COUNT must be a positive integer"

E2E_TMP="$(mktemp -d "${TMPDIR:-/tmp}/instantwm-e2e.XXXXXX")"
SOCKET_PATH="$E2E_TMP/instantwm.sock"
WM_LOG="${WM_LOG:-$E2E_TMP/instantwm.log}"
OVERLAP_FILE="$E2E_TMP/overlap.txt"
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

cd "$RUST_DIR"
cargo build --quiet --bin instantwm --bin instantwmctl

INSTANTWM_SOCKET_BIND="$SOCKET_PATH" \
INSTANTWM_AUTOSTART=0 \
INSTANTWM_WL_AUTOSTART=0 \
INSTANTWM_WL_AUTOSPAWN=0 \
INSTANTWM_TEST=1 \
  setsid --wait timeout 45s ./target/debug/instantwm --backend wayland >"$WM_LOG" 2>&1 &
wm_pid=$!

ready=0
for _ in $(seq 1 "$STARTUP_RETRIES"); do
  if run_ctl status >/dev/null 2>&1; then
    ready=1
    break
  fi
  kill -0 "$wm_pid" >/dev/null 2>&1 || break
  sleep "$STARTUP_SLEEP"
done
(( ready == 1 )) || die "instantWM IPC did not become ready; see $WM_LOG"

write_window_ids "$E2E_TMP/ids-initial.txt"
initial_count="$(wc -l <"$E2E_TMP/ids-initial.txt" | tr -d ' ')"

for idx in $(seq 1 "$SPAWN_COUNT"); do
  cmd="$(choose_spawn_cmd "$idx")"
  spawn_response="$(run_ctl spawn "$cmd")"
  printf '%s\n' "$spawn_response" >"$E2E_TMP/spawn-$idx.txt"
  if [[ "$spawn_response" =~ ^pid=([0-9]+)$ ]]; then
    client_pgids+=("${BASH_REMATCH[1]}")
  else
    die "could not parse spawned client PID from '$spawn_response'"
  fi
done

run_ctl test wait windows "$((initial_count + SPAWN_COUNT))" --timeout-ms 7000 >/dev/null
write_window_ids "$E2E_TMP/ids-after-spawn.txt"
comm -13 "$E2E_TMP/ids-initial.txt" "$E2E_TMP/ids-after-spawn.txt" >"$E2E_TMP/ids-new.txt"
new_count="$(wc -l <"$E2E_TMP/ids-new.txt" | tr -d ' ')"
if (( new_count < SPAWN_COUNT )); then
  die "expected at least $SPAWN_COUNT new windows, got $new_count"
fi

: >"$E2E_TMP/geometries.txt"
while read -r id; do
  run_ctl --json window info "$id" | python3 -c '
import json, sys
window = json.load(sys.stdin)
geometry = window["geometry"]
print(window["id"], geometry["x"], geometry["y"], geometry["width"], geometry["height"])
' >>"$E2E_TMP/geometries.txt"
done <"$E2E_TMP/ids-new.txt"

if ! awk 'NF >= 5 && $3 > 0 {found=1} END{exit(found?0:1)}' "$E2E_TMP/geometries.txt"; then
  die "no window y-offset detected; the bar may overlap the tiled area"
fi
if ! awk -v expected="$SPAWN_COUNT" -v overlap_file="$OVERLAP_FILE" '
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
      # Tiled clients may overlap slightly during animation, but covering more
      # than 70% of the smaller window indicates a broken layout geometry.
      if (ratio > 0.70) {
        print "Excessive overlap between windows " id[i] " and " id[j] ": " ratio > "/dev/stderr";
        bad = 1;
      }
    }
  }
  if (bad) exit 1;
  print "max_overlap_ratio=" max_ratio > overlap_file;
}
' "$E2E_TMP/geometries.txt"; then
  die "geometry sanity check failed; see $E2E_TMP/geometries.txt"
fi

first_id="$(head -n1 "$E2E_TMP/ids-new.txt")"
run_ctl test window focus "$first_id"
run_ctl test window tag "$first_id" 2
actual_tags="$(run_ctl --json window info "$first_id" | python3 -c '
import json, sys
print(json.load(sys.stdin)["tags"])
')"
assert_eq 2 "$actual_tags" "window tag assignment"
run_ctl test window tag "$first_id" 1
run_ctl test window mode "$first_id" floating
run_ctl test window mode "$first_id" tiled
run_ctl test pointer path --normalized --duration-ms 100 --hz 20 \
  0.1,0.01 0.9,0.01 >/dev/null

while read -r id; do
  run_ctl window close "$id" >/dev/null
done <"$E2E_TMP/ids-new.txt"

run_ctl test wait windows "$initial_count" --exact --timeout-ms 5000 >/dev/null
write_window_ids "$E2E_TMP/ids-final.txt"
if ! diff -u "$E2E_TMP/ids-initial.txt" "$E2E_TMP/ids-final.txt" >"$E2E_TMP/id-diff.txt"; then
  die "window IDs differ after close; see $E2E_TMP/id-diff.txt"
fi

if grep -Eq "thread .* panicked at|instantwm.*negative size" "$WM_LOG"; then
  die "WM log contains a compositor error; see $WM_LOG"
fi

echo "PASS: spawned=$new_count $(<"$OVERLAP_FILE")"
