/* See LICENSE file for copyright and license details. */

#include "config.h"
#include "instantwm.h"

/* `rules` must have exactly one definition in the entire program.
 * Keep it here (in a single .c file) and only declare it as `extern`
 * in headers.
 */
const Rule rules[] = {
    /* xprop(1):
     *	WM_CLASS(STRING) = instance, class
     *	WM_NAME(STRING) = title
     */
    /* class                        instance  title  tags mask  isfloating
       monitor */
    {"Pavucontrol", NULL, NULL, 0, 1, -1},
    {"Onboard", NULL, NULL, 0, 1, -1},
    {"floatmenu", NULL, NULL, 0, 1, -1},
    {"Welcome.py", NULL, NULL, 0, 1, -1},
    {"Pamac-installer", NULL, NULL, 0, 1, -1},
    {"xpad", NULL, NULL, 0, 1, -1},
    {"Guake", NULL, NULL, 0, 1, -1},
    {"instantfloat", NULL, NULL, 0, 2, -1},
    {scratchpadname, NULL, NULL, 0, 4, -1},
    {"kdeconnect.daemon", NULL, NULL, 0, 3, -1},
    {"Panther", NULL, NULL, 0, 3, -1},
    {"org-wellkord-globonote-Main", NULL, NULL, 0, 1, -1},
    {"Peek", NULL, NULL, 0, 1, -1},
};

/* variables calculated from config.h arrays */
unsigned int tagmask = TAGMASK;
int numtags = LENGTH(tags);
size_t keys_len = LENGTH(keys);
size_t dkeys_len = LENGTH(dkeys);
size_t commands_len = LENGTH(commands);
size_t buttons_len = LENGTH(buttons);
size_t layouts_len = LENGTH(layouts);
size_t rules_len = LENGTH(rules);
size_t fonts_len = LENGTH(fonts);
