#!/bin/sh
# INSTANTWM_LOG controls instantWM's built-in stderr logger (see src/logging.rs).
export INSTANTWM_LOG=debug
export INSTANTWM_AUTOSTART=0
# The DRM backend takes over the TTY, so stderr must go to a file to be readable.
exec instantwm --backend drm 2>>"$HOME/.instantwm.log"
