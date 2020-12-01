#!/usr/bin/env bash
description="$0 <command> <args>...

Really basic tool to send commands to instantWM.

Commands:
    help                     Display this help text
    overlay
    tag
    animated
    alttab
    layout <number>|<name>   Change window layout to given argument, e.g. $0 layout monocle
    prefix
    focusfollowsmouse        Control if window focus will change with mouse movement
    allttag
    hidetags"


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

