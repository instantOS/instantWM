#!/bin/bash

THEME=themes/${1:-arc}.theme
echo "$THEME"
grep -q 'size' <"$THEME" || { echo "theme not valid" && exit 1; }

replacetheme() {
    themecolor=$(grep "$1" <"$THEME" | head -1)
    echo "'s~.*'"$1"'.*~'"$themecolor"'~'"
    sed -i 's~.*'"$1"'.*~'"${themecolor/\*/\\\*}"'~' config.def.h
}

replacetheme '\*fonts'
replacetheme "r col_gray1"
replacetheme "r col_gray2"
replacetheme "r col_gray3"
replacetheme "r col_gray4"
replacetheme "r col_gray5"
replacetheme "r col_cyan"
