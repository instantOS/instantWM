#!/usr/bin/env bash

make clean &>/dev/null

THEME=themes/${1:-dracula}.theme
echo "$THEME"
grep -q 'size' <$THEME || { echo "theme not valid" && exit 1; }

replacetheme() {
    themecolor=$(grep "$1" <$THEME)
    sed -i 's/.*'"$1"'.*/'"$themecolor"'/' config.def.h
}

replacetheme '*fonts[]'
dmenufont 'dmenufont[]'
replacetheme dmenufont
replacetheme col_gray1
replacetheme col_gray2
replacetheme col_gray3
replacetheme col_gray4
replacetheme col_gray5

if [ -z "$2" ]; then
    make
    sudo make install
fi
