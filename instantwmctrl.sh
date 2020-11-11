#!/usr/bin/env bash
description="$0 <command> <args>...

Really basic tool to send commands to instantWM.

Commands:
    help                     Display this help text
    layout <number>|<name>   Change window layout to given argument, e.g. $0 layout monocle"

main() {
    case $1 in
        help) usage -h ;;

        layout)
            if [[ $2 =~ ^[0-8]$ ]]; then # between zero and eight?
                layout=$2
            else
                declare -A layouts=(
                ["tile"]=0 ['grid']=1 ['float']=2 
                ['monocle']=3 ['tcl']=4 ['deck']=5
                ['overview']=6 ['bstack']=7 ['bstackhoriz']=8
                )
                layout=${layouts[$2]}
                [ -z "$layout" ] &&
                { echo "Error: Unknown layout '$2'"; exit 1; }
            fi 
            xsetroot -name "c;:;layout;$layout"
            exit
        ;;
    esac
    xsetroot -name "c;:;$1;$2"
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

