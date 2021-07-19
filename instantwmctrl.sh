#!/usr/bin/env bash
description="$0 <command> <args>...

Really basic tool to send commands to instantWM.

Commands:
    help                     Display this help text
    overlay                  Toggle overlay (Super + Ctrl + W to define a widnow as overlay)
    warpfocus                Warp mouse to currently focussed window
    tag <number>             Switch to tag described by <number>
    animated                 Toggle animations
    alttab                   
    layout <number>|<name>   Change window layout to given argument, e.g. $0 layout monocle
    prefix                   Set action prefix
    focusfollowsmouse        Toggle window focus will change with mouse movement
    focusfollowsfloatmouse   As above but only for flowting windows
    focusmon                 Switch focus to other monitor
    focusnmon                Focus monitor with index n
    tagmon                   Move window to other monitor
    followmon                Two above combined
    border <number>          Set window border width to <number>
    alttag                   Display tag symbols instead of numbers
    hidetags 0|1             Hide tags that have no windows on current monitor (0 means hide)
    nametag <name>           change the name/icon of the current tag
    resetnametag             reset all tag names to default"
# See config.def.c and look for "Xcommand commands"

main() {
    case $1 in
        help) usage -h ;;
        layout) layout "$2"; exit ;;
    esac
    xsetroot -name "c;:;$1;$2"
}

layout() {
    if [[ $1 =~ ^[0-8]$ ]]; then # between zero and eight
        layout=$1
    else
        declare -A layouts=(
            ["tile"]=0 ['grid']=1 ['float']=2 
            ['monocle']=3 ['tcl']=4 ['deck']=5
            ['overview']=6 ['bstack']=7 ['bstackhoriz']=8
        )
        layout=${layouts[$1]}
        [ -z "$layout" ] &&
            { echo "Error: Unknown layout '$1'"; exit 1; }
    fi 
    xsetroot -name "c;:;layout;$layout"
}

usage() {
    for itm in "$@"; do
    if [[ "$itm" =~ ^(-h|--help|-help|-\?)$ ]]; then
        1>&2 echo "Usage: $description"; exit 0;
    fi
    done
}

if [ "$0" = "$BASH_SOURCE" ]; then
    usage "$@"
    main "$@"
fi

