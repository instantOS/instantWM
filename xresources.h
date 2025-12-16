/* See LICENSE file for copyright and license details. */

#ifndef XRESOURCES_H
#define XRESOURCES_H

#include "instantwm.h"

void load_xresources(void);
void list_xresources(void);
void resource_load(XrmDatabase db, char *name, enum resource_type rtype,
                   void *dst);
void verifytagsxres(void);

#endif
