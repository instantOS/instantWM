#!/usr/bin/env bash

# compile and install instantWM

SUPERTOOL="sudo"
if [[ -x /usr/bin/doas ]] && [[ -s /etc/doas.conf ]] ; then
    SUPERTOOL="doas"
fi

make clean &>/dev/null

if [ -z "$2" ]; then
    $SUPERTOOL make install
fi
