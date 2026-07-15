#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CTL="${PROFILE_CTL:-$ROOT_DIR/target/profiling/instantwmctl}"
WINDOWS="${PROFILE_WINDOWS:-4}"
STEP_SLEEP="${PROFILE_STEP_SLEEP:-0.20}"
WORKLOAD="${PROFILE_WORKLOAD:-standard}"

[[ "$WINDOWS" =~ ^[1-9][0-9]*$ ]] || {
  echo "PROFILE_WINDOWS must be a positive integer" >&2
  exit 1
}

run_ctl() {
  "$CTL" "$@"
}

choose_app() {
  if [[ -n "${PROFILE_APP_CMD:-}" ]]; then
    printf '%s' "$PROFILE_APP_CMD"
  elif command -v foot >/dev/null 2>&1; then
    printf '%s' 'foot'
  elif command -v weston-terminal >/dev/null 2>&1; then
    printf '%s' 'weston-terminal'
  elif command -v gtk4-demo >/dev/null 2>&1; then
    printf '%s' 'gtk4-demo'
  elif command -v gtk3-demo >/dev/null 2>&1; then
    printf '%s' 'gtk3-demo'
  elif command -v xmessage >/dev/null 2>&1; then
    printf '%s' 'xmessage instantWM-profile'
  else
    return 1
  fi
}

app="$(choose_app)" || {
  echo "No test client found; set PROFILE_APP_CMD to a command that opens a window" >&2
  exit 1
}

if [[ "$WORKLOAD" == "stress" ]]; then
  stress_app="${PROFILE_STRESS_APP_CMD:-vkcube --wsi wayland --suppress_popups}"
  command -v "${stress_app%% *}" >/dev/null 2>&1 || {
    echo "Stress workload requires vkcube or PROFILE_STRESS_APP_CMD" >&2
    exit 1
  }
  echo "stress_app=$stress_app"
  run_ctl spawn "$stress_app"
  static_windows=$((WINDOWS - 1))
else
  static_windows=$WINDOWS
fi

echo "workload=$WORKLOAD app=$app static_windows=$static_windows"
for _ in $(seq 1 "$static_windows"); do
  run_ctl spawn "$app"
  sleep "$STEP_SLEEP"
done

# Wait for clients to map before repeatedly exercising layout and rendering work.
sleep 1
run_ctl toggle animated on
layouts=(tile grid monocle deck bottom-stack horizgrid gaplessgrid bstackhoriz floating)
deadline=$((SECONDS + ${PROFILE_DURATION:-20} - 2))
while (( SECONDS < deadline )); do
  for layout in "${layouts[@]}"; do
    run_ctl layout "$layout"
    run_ctl update-status "profile:$layout"
    sleep "$STEP_SLEEP"
    (( SECONDS >= deadline )) && break
  done
done
