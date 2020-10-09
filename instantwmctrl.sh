#!/usr/bin/env dash

# really basic tool to send commands to instantWM

case $1 in
layout)
    if [ "$2" -eq "$2" ]; then
        LAYOUT="$2"
    else
        case $2 in
        tile)
            LAYOUT=0
            ;;
        grid)
            LAYOUT=1
            ;;
        float)
            LAYOUT=2
            ;;
        monocle)
            LAYOUT=3
            ;;
        tcl)
            LAYOUT=4
            ;;
        deck)
            LAYOUT=5
            ;;
        overviewlayout)
            LAYOUT=6
            ;;
        bstack)
            LAYOUT=7
            ;;
        bstackhoriz)
            LAYOUT=8
            ;;
        esac
    fi
    xsetroot -name "c;:;layout;$LAYOUT"
    exit
    ;;
esac

xsetroot -name "c;:;$1;$2"
