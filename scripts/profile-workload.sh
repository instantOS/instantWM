#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CTL="${PROFILE_CTL:-$ROOT_DIR/target/profiling/instantwmctl}"
STEP_SLEEP="${PROFILE_STEP_SLEEP:-0.20}"
WORKLOAD="${PROFILE_WORKLOAD:-standard}"
if [[ "$WORKLOAD" == "standard" ]]; then
  WINDOWS="${PROFILE_WINDOWS:-12}"
else
  WINDOWS="${PROFILE_WINDOWS:-4}"
fi
TAGS="${PROFILE_TAGS:-4}"
WORKLOAD_STARTED=$SECONDS

[[ "$WINDOWS" =~ ^[1-9][0-9]*$ ]] || {
  echo "PROFILE_WINDOWS must be a positive integer" >&2
  exit 1
}
[[ "$TAGS" =~ ^[1-9][0-9]*$ ]] || {
  echo "PROFILE_TAGS must be a positive integer" >&2
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
  active_app="${PROFILE_ACTIVE_APP_CMD:-vkcube --wsi wayland --suppress_popups}"
  command -v "${active_app%% *}" >/dev/null 2>&1 || {
    echo "Standard workload requires vkcube or PROFILE_ACTIVE_APP_CMD" >&2
    exit 1
  }
  static_windows=$((WINDOWS - 1))
else
  static_windows=$WINDOWS
fi

if [[ "$WORKLOAD" == "standard" ]]; then
  configured_tags="$(run_ctl --json status | python3 -c 'import json,sys; print(json.load(sys.stdin)["tags"])')"
  (( TAGS <= configured_tags )) || {
    echo "PROFILE_TAGS=$TAGS exceeds the $configured_tags configured tags" >&2
    exit 1
  }
  (( static_windows >= TAGS )) || {
    echo "standard workload needs at least PROFILE_TAGS + 1 total windows" >&2
    exit 1
  }

  echo "workload=$WORKLOAD app=$app static_windows=$static_windows tags=$TAGS"
  mapped=0
  base_per_tag=$((static_windows / TAGS))
  extra=$((static_windows % TAGS))
  for tag in $(seq 1 "$TAGS"); do
    run_ctl tag view "$tag"
    on_tag=$base_per_tag
    if (( tag <= extra )); then
      on_tag=$((on_tag + 1))
    fi
    for _ in $(seq 1 "$on_tag"); do
      run_ctl spawn "$app"
    done
    mapped=$((mapped + on_tag))
    run_ctl test wait windows "$mapped" --timeout-ms 10000
  done

  # Keep an actively rendering client on another tag, as in a normal desktop
  # with video or animation on a neighboring workspace.
  run_ctl tag view 2
  echo "active_app=$active_app tag=2"
  run_ctl spawn "$active_app"
  run_ctl test wait windows "$WINDOWS" --timeout-ms 10000
  run_ctl tag view 1
else
  echo "workload=$WORKLOAD app=$app static_windows=$static_windows"
  for _ in $(seq 1 "$static_windows"); do
    run_ctl spawn "$app"
  done
  run_ctl test wait windows "$WINDOWS" --timeout-ms 10000
fi

run_ctl toggle animated on

if [[ "$WORKLOAD" == "standard" ]]; then
  # Make one window floating over the tiled clients. Coordinates are relative
  # to the focused monitor, and scale with its current resolution.
  floating_id="$(run_ctl --json window list | python3 -c '
import json, sys
windows = json.load(sys.stdin)
tag_one = [window for window in windows if window["tags"] & 1]
print(tag_one[0]["id"] if tag_one else "")
')"
  read -r monitor_width monitor_height < <(run_ctl --json monitor list | python3 -c '
import json, sys
monitor = json.load(sys.stdin)[0]
print(monitor["width"], monitor["height"])
')
  if [[ -n "$floating_id" ]]; then
    run_ctl test window focus "$floating_id"
    run_ctl test window mode "$floating_id" floating
    run_ctl window resize "$floating_id" \
      --x $((monitor_width / 5)) --y $((monitor_height / 6)) \
      --width $((monitor_width * 3 / 5)) --height $((monitor_height * 2 / 3))
  fi
fi

layouts=(tile grid monocle deck bottom-stack horizgrid gaplessgrid bstackhoriz floating)
deadline=$((WORKLOAD_STARTED + ${PROFILE_DURATION:-20} - 1))
layout_iteration=0
while (( SECONDS < deadline )); do
  for layout in "${layouts[@]}"; do
    pointer_pid=""
    if [[ "$WORKLOAD" == "standard" ]]; then
      # Exercise the real compositor hit-testing path concurrently with layout,
      # tag, animation, and status work. Both bar edges are visited so this is
      # valid for top- and bottom-bar configurations.
      run_ctl test pointer path --normalized --duration-ms 450 --hz 40 \
        0.03,0.005 0.97,0.005 0.97,0.995 0.03,0.995 0.50,0.50 &
      pointer_pid=$!
    fi
    run_ctl layout "$layout"
    run_ctl update-status "profile:$layout"
    if [[ "$WORKLOAD" == "standard" ]]; then
      next_tag=$((layout_iteration % TAGS + 1))
      layout_iteration=$((layout_iteration + 1))
      run_ctl tag view "$next_tag"
      sleep "$STEP_SLEEP"
      run_ctl tag view 1
    fi
    sleep "$STEP_SLEEP"
    if [[ -n "$pointer_pid" ]]; then
      wait "$pointer_pid"
    fi
    (( SECONDS >= deadline )) && break
  done
done
