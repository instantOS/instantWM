/* See LICENSE file for copyright and license details. */

#ifndef MONITORS_H
#define MONITORS_H

#include "instantwm.h"

void cleanupmon(Monitor *mon);
Monitor *dirtomon(int dir);
void followmon(const Arg *arg);
void focusmon(const Arg *arg);
void focusnmon(const Arg *arg);
Monitor *recttomon(int x, int y, int w, int h);
void sendmon(Client *c, Monitor *m);
Monitor *wintomon(Window w);

#endif
