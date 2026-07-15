#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CTL="${PROFILE_CTL:-$ROOT_DIR/target/profiling/instantwmctl}"
WINDOWS="${PROFILE_WINDOWS:-4}"
STEP_SLEEP="${PROFILE_STEP_SLEEP:-0.20}"
WORKLOAD="${PROFILE_WORKLOAD:-standard}"
WORKLOAD_STARTED=$SECONDS

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

if [[ "$WORKLOAD" == "standard" ]]; then
  active_app="${PROFILE_ACTIVE_APP_CMD:-${PROFILE_STRESS_APP_CMD:-vkcube --wsi wayland --suppress_popups}}"
  command -v "${active_app%% *}" >/dev/null 2>&1 || {
    echo "Standard workload requires vkcube or PROFILE_ACTIVE_APP_CMD" >&2
    exit 1
  }
  static_windows=$((WINDOWS - 1))
else
  static_windows=$WINDOWS
fi

echo "workload=$WORKLOAD app=$app static_windows=$static_windows"
for _ in $(seq 1 "$static_windows"); do
  run_ctl spawn "$app"
  sleep "$STEP_SLEEP"
done

if [[ "$WORKLOAD" == "standard" ]]; then
  # Keep an actively rendering client on another tag, as in a normal desktop
  # with video or animation on a neighboring workspace.
  run_ctl tag view 2
  echo "active_app=$active_app tag=2"
  run_ctl spawn "$active_app"
  sleep 1
  run_ctl tag view 1
fi

# Wait for clients to map before repeatedly exercising layout and rendering work.
sleep 1
run_ctl toggle animated on

if [[ "$WORKLOAD" == "standard" ]]; then
  # Make one window floating over the tiled clients. Coordinates are relative
  # to the focused monitor, and scale with its current resolution.
  floating_id="$(run_ctl --json window list | python3 -c '
import json, sys
windows = json.load(sys.stdin)
print(windows[0]["id"] if windows else "")
')"
  read -r monitor_width monitor_height < <(run_ctl --json monitor list | python3 -c '
import json, sys
monitor = json.load(sys.stdin)[0]
print(monitor["width"], monitor["height"])
')
  if [[ -n "$floating_id" ]]; then
    run_ctl window resize "$floating_id" \
      --x $((monitor_width / 5)) --y $((monitor_height / 6)) \
      --width $((monitor_width * 3 / 5)) --height $((monitor_height * 2 / 3))
  fi
fi

layouts=(tile grid monocle deck bottom-stack horizgrid gaplessgrid bstackhoriz floating)
deadline=$((WORKLOAD_STARTED + ${PROFILE_DURATION:-20} - 1))
while (( SECONDS < deadline )); do
  for layout in "${layouts[@]}"; do
    run_ctl layout "$layout"
    run_ctl update-status "profile:$layout"
    if [[ "$WORKLOAD" == "standard" ]]; then
      run_ctl tag view 2
      sleep "$STEP_SLEEP"
      run_ctl tag view 1
    fi
    sleep "$STEP_SLEEP"
    (( SECONDS >= deadline )) && break
  done
done
