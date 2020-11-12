#!/usr/bin/env bash

# compile and install instantWM

make clean &>/dev/null

if [ -z "$2" ]; then
    rm config.h &>/dev/null
    make
    sudo make install
fi
