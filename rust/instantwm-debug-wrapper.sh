#!/bin/sh
export RUST_LOG=debug
export INSTANTWM_AUTOSTART=0
exec instantwm-rs --backend drm 2>>"$HOME/.instantwm.log"
