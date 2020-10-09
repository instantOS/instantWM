#!/usr/bin/env bash

# really basic tool to send commands to instantWM

case $1 in
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
