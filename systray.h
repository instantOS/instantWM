/* See LICENSE file for copyright and license details. */

#ifndef SYSTRAY_H
#define SYSTRAY_H

#include "instantwm.h"

unsigned int getsystraywidth(void);
void removesystrayicon(Client *i);
void updatesystrayicongeom(Client *i, int w, int h);
void updatesystrayiconstate(Client *i, XPropertyEvent *ev);
void updatesystray(void);
Client *wintosystrayicon(Window w);
Monitor *systraytomon(Monitor *m);

#endif
