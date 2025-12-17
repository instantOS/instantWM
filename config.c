/* See LICENSE file for copyright and license details. */

#include "instantwm.h"
#include "layouts.h"
#include "push.h"
#include "config.h"

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
