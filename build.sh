#!/usr/bin/env bash

# compile and install instantWM

SUPERTOOL="sudo"
if [[ -x /usr/bin/doas ]] && [[ -s /etc/doas.conf ]] ; then
    SUPERTOOL="doas"
fi

make clean &>/dev/null

if [ -z "$2" ]; then
    mv config.h "$(mktemp --tmpdir instantwmcfg_XXXX.h)" &>/dev/null && 
        echo "Existing config.h moved to /tmp/" 1>&2
    $SUPERTOOL make install
fi
