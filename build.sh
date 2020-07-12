#!/usr/bin/env bash

# ./theme.sh "$1"

make clean &>/dev/null

if [ -z "$2" ]; then
    rm config.h &>/dev/null
    make
    sudo make install
fi
