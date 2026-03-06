#!/bin/sh
export RUST_LOG=debug
exec instantwm-rs --backend drm 2>>"$HOME/.instantwm.log"
