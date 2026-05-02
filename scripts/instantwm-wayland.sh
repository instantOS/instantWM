#!/bin/sh

echo "starting instantWM"
export INSTANTWM_AUTOSTART=1
exec instantwm --backend drm
