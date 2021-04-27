#/usr/bin/dash

CONFIG_FILE="config.def.h"
OUTPUT="keys"

section_prefix="--"

change_modifiers='s/MODKEY/Super/;s/ShiftMask/Shift/;s/Mod1Mask/Alt/;s/ControlMask/Control/'
change_KEY="s/KEY/[n]/"
change_key_prefix='s/[A-Z0-9]*XK_//g;s/Audio//;s/Volume/ Volume/;s/MonBrightness/Brightness /'
change_buttons='s/Button1/Left Click/;s/Button2/Middle Click/;s/Button3/Right Click/;s/Button4/Scroll Up/;s/Button5/Scroll Down/'
change_Clk_area='s/ClkClientWin/Window/;s/ClkCloseButton/Close Button/;s/ClkLtSymbol/Layout Symbol/;s/ClkRootWin/Wallpaper/;s/ClkShutDown/Power Icon/;s/ClkSideBar/Right Side/;s/ClkStartMenu/Logo Button/;s/ClkStatusText/Bar Status/;s/ClkTagBar/Tags/;s/ClkWinTitle/Window Title/'

only_dkeys='1,/Key dkeys/d;/};/,$d'
only_keys='1,/Key keys/d;/};/,$d'
only_tagkeys='1,/^#define TAGKEYS/d;/^$/,$d'
only_mouse='1,/Button/d;/};/,$d'

remove_semicolons='/;/d'
remove_backslash='s/\s*\*\s*\/\s*\\\s*$//'
remove_define='/#/d'
remove_comments='/\/\*/d'
remove_whitespace='s/^\s*//g;s/\s*$//g'
remove_braces='s/\s*[{}]\s*//g'
remove_zero='s/0\s*+\s*//'
remove_TAGKEYS='/TAGKEYS/d'
remove_empty_lines='/^$/d'
remove_Clk_area='s/Clk[A-Za-z]* + //'

comma_to_plus='s/\s*,\s*/ + /'
bar_to_plus='s/\s*|\s*/ + /g'

clear_to_comment='s/,.*\/\/\s*/: /'
clear_to_tagkey_comment='s/,.*\/\*\s*/: /'

(
for i in $(sed "$only_mouse;$remove_semicolons;$remove_comments;$remove_whitespace;$remove_empty_lines;$remove_braces"';s/,.*//' < $CONFIG_FILE | sort | uniq); do
	echo "$section_prefix"$i$'\n' | sed "$change_Clk_area"
	sed "$remove_whitespace;$remove_comments;$remove_semicolons;$remove_braces;$change_modifiers;$change_buttons;$bar_to_plus;$comma_to_plus;$comma_to_plus;$remove_zero;$clear_to_comment" < $CONFIG_FILE | grep "$i" | sed "s/^$i+//;$remove_Clk_area"
	echo
done

echo "$section_prefix"$'Tag Keys\n'
sed "$only_tagkeys;$remove_define;$remove_empty_lines;$remove_whitespace;$remove_semicolons;$remove_braces;$change_modifiers;$comma_to_plus;$change_KEY;$bar_to_plus;$remove_backslash;$clear_to_tagkey_comment" < $CONFIG_FILE

echo "$section_prefix"$'Keys\n'
sed "$only_keys;$remove_empty_lines;$remove_whitespace;$remove_comments;$remove_semicolons;$remove_braces;$remove_TAGKEYS;$change_modifiers;$comma_to_plus;$bar_to_plus;$remove_zero;$change_key_prefix;$clear_to_comment" < $CONFIG_FILE

echo $'\n'"$section_prefix"$'Desktop Keys\n'
sed "$only_dkeys;$remove_empty_lines;$remove_whitespace;$remove_comments;$remove_semicolons;$remove_braces;$remove_TAGKEYS;$change_modifiers;$comma_to_plus;$bar_to_plus;$remove_zero;$change_key_prefix;$clear_to_comment" < $CONFIG_FILE
) > $OUTPUT
