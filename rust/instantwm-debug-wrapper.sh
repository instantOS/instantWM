#!/bin/sh
export RUST_LOG=debug
exec instantwm-rs --backend wayland 2>>"$HOME/.instantwm.log"
