#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DURATION="${1:-20}"
WORKLOAD="${2:-standard}"
PROFILE_FREQ="${PROFILE_FREQ:-199}"
PROFILE_DIR="${PROFILE_DIR:-$ROOT_DIR/target/profiles/$(date +%Y%m%d-%H%M%S)}"
PERF_BIN="${PERF:-perf}"
WM_BIN="$ROOT_DIR/target/profiling/instantwm"
CTL_BIN="$ROOT_DIR/target/profiling/instantwmctl"
SOCKET="/tmp/instantwm-$(id -u).sock"

die() {
  echo "profile: $*" >&2
  exit 1
}

socket_is_live() {
  python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX)
s.settimeout(0.5)
s.connect(sys.argv[1])
' "$1" >/dev/null 2>&1
}

[[ "$DURATION" =~ ^[1-9][0-9]*$ ]] || die "duration must be a positive number of seconds"
[[ "$WORKLOAD" == "standard" || "$WORKLOAD" == "stress" || "$WORKLOAD" == "manual" ]] || die "workload must be 'standard', 'stress', or 'manual'"
command -v "$PERF_BIN" >/dev/null 2>&1 || die "perf is required (install the package matching the running kernel)"
[[ -x "$WM_BIN" && -x "$CTL_BIN" ]] || die "profiling binaries are missing; run 'just profile-build'"

paranoid="$(cat /proc/sys/kernel/perf_event_paranoid 2>/dev/null || echo unknown)"
if [[ "$paranoid" =~ ^-?[0-9]+$ ]] && (( paranoid > 2 )); then
  die "kernel.perf_event_paranoid=$paranoid; run 'just profile-permissions' once, then retry"
fi

if ! perf_preflight="$("$PERF_BIN" stat --all-user --event cpu-clock:u -- true 2>&1)"; then
  perf_preflight="${perf_preflight//$'\n'/ }"
  die "perf cannot open the required cpu-clock:u event; run 'just profile-permissions' and retry. perf said: $perf_preflight"
fi

# A DRM compositor must own the seat, so sharing the default IPC socket is
# almost certainly an accidentally running instantWM instance.
if [[ -S "$SOCKET" ]]; then
  if socket_is_live "$SOCKET"; then
    die "instantWM is already running on $SOCKET; stop it before a DRM capture"
  fi
  rm -f "$SOCKET"
fi

mkdir -p "$PROFILE_DIR"
PROFILE_DIR="$(cd "$PROFILE_DIR" && pwd)"
PERF_DATA="$PROFILE_DIR/perf.data"
WM_LOG="$PROFILE_DIR/instantwm.log"
WORKLOAD_LOG="$PROFILE_DIR/workload.log"
STARTED_AT="$(date --iso-8601=seconds)"
GIT_REVISION="$(git -C "$ROOT_DIR" rev-parse HEAD 2>/dev/null || echo unknown)"
if [[ -n "$(git -C "$ROOT_DIR" status --porcelain 2>/dev/null)" ]]; then
  GIT_DIRTY=true
else
  GIT_DIRTY=false
fi

if [[ ! -t 0 ]]; then
  echo "profile: warning: stdin is not a TTY; DRM/libseat normally needs an active local TTY" >&2
fi

cleanup() {
  if [[ -n "${gpu_pid:-}" ]] && kill -0 "$gpu_pid" 2>/dev/null; then
    kill -INT "$gpu_pid" 2>/dev/null || true
    wait "$gpu_pid" 2>/dev/null || true
  fi
  if [[ -n "${perf_pid:-}" ]] && kill -0 "$perf_pid" 2>/dev/null; then
    kill -INT "$perf_pid" 2>/dev/null || true
    wait "$perf_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

echo "profile: recording ${DURATION}s into $PROFILE_DIR"
INSTANTWM_AUTOSTART=0 \
INSTANTWM_WL_AUTOSTART=0 \
INSTANTWM_WL_AUTOSPAWN=0 \
RUST_LOG="${RUST_LOG:-warn}" \
"$PERF_BIN" record \
  --all-user \
  --event cpu-clock:u \
  --freq "$PROFILE_FREQ" \
  --call-graph fp \
  --output "$PERF_DATA" \
  -- "$WM_BIN" --backend drm >"$WM_LOG" 2>&1 &
perf_pid=$!

wm_pid=""
for _ in $(seq 1 100); do
  wm_pid="$(pgrep -P "$perf_pid" -x instantwm 2>/dev/null | head -n1 || true)"
  [[ -n "$wm_pid" ]] && break
  kill -0 "$perf_pid" 2>/dev/null || break
  sleep 0.05
done
[[ -n "$wm_pid" ]] || die "instantWM did not start; see $WM_LOG"

ready=0
for _ in $(seq 1 200); do
  if INSTANTWM_SOCKET="$SOCKET" "$CTL_BIN" status >/dev/null 2>&1; then
    ready=1
    break
  fi
  kill -0 "$perf_pid" 2>/dev/null || break
  sleep 0.05
done
if (( ! ready )); then
  die "instantWM IPC did not become ready; see $WM_LOG (an active TTY is usually required)"
fi

printf '{\n  "schema_version": 1,\n  "started_at": "%s",\n  "duration_seconds": %s,\n  "frequency_hz": %s,\n  "workload": "%s",\n  "pid": %s,\n  "backend": "drm",\n  "cargo_profile": "profiling",\n  "perf_event": "cpu-clock:u",\n  "perf_event_scope": "user-space CPU samples",\n  "git_revision": "%s",\n  "git_dirty": %s\n}\n' \
  "$STARTED_AT" "$DURATION" "$PROFILE_FREQ" "$WORKLOAD" "$wm_pid" \
  "$GIT_REVISION" "$GIT_DIRTY" >"$PROFILE_DIR/metadata.json"

python3 "$ROOT_DIR/scripts/profile-gpu.py" "$wm_pid" "$PROFILE_DIR" \
  "${GPU_SAMPLE_MS:-100}" >"$PROFILE_DIR/gpu-monitor.log" 2>&1 &
gpu_pid=$!

if [[ "$WORKLOAD" == "standard" || "$WORKLOAD" == "stress" ]]; then
  INSTANTWM_SOCKET="$SOCKET" PROFILE_DURATION="$DURATION" PROFILE_WORKLOAD="$WORKLOAD" \
    bash "$ROOT_DIR/scripts/profile-workload.sh" >"$WORKLOAD_LOG" 2>&1 &
  workload_pid=$!
  echo "profile: scripted workload started; recording for ${DURATION}s (manual input is also welcome)"
else
  echo "profile: manual capture active for ${DURATION}s; open, move, and interact with windows now"
fi

sleep "$DURATION" &
sleep_pid=$!
wait "$sleep_pid"
echo "profile: capture duration elapsed; finalizing perf data"
kill -INT "$gpu_pid" 2>/dev/null || true
wait "$gpu_pid" || true
gpu_pid=""
kill -INT "$perf_pid" 2>/dev/null || true
wait "$perf_pid" || true
perf_pid=""
if [[ -n "${workload_pid:-}" ]]; then
  wait "$workload_pid" 2>/dev/null || true
fi
trap - EXIT INT TERM

echo "profile: recording complete; generating agent reports"
python3 "$ROOT_DIR/scripts/profile-report.py" "$PROFILE_DIR"
ln -sfn "$(basename "$PROFILE_DIR")" "$ROOT_DIR/target/profiles/latest"
echo "profile: complete; start with $PROFILE_DIR/summary.md"
